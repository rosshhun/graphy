use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, QueryParser};
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, SingleSegmentIndexWriter, TantivyDocument};
use tracing::info;

use graphy_core::{CodeGraph, NodeKind};

#[cfg(feature = "vectors")]
pub mod vector;

#[cfg(feature = "vectors")]
pub use vector::{default_vector_path, SemanticResult, VectorIndex};

/// Search result with score and matched node info.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub symbol_id: u64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: u32,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub language: String,
    pub score: f32,
}

/// The search index backed by tantivy.
///
/// Uses `SingleSegmentIndexWriter` for full rebuilds (zero thread overhead)
/// and a short-lived `IndexWriter` only for incremental updates.
pub struct SearchIndex {
    index: Index,
    schema: Schema,
    /// Path for persistent indexes (None for in-memory).
    persist_path: Option<PathBuf>,
    /// Guards write operations so only one caller writes at a time.
    write_lock: Mutex<()>,
    /// Persistent reader — reloaded after commits to pick up new segments.
    reader: IndexReader,
    // Field handles
    f_id: Field,
    f_name: Field,
    f_kind: Field,
    f_file_path: Field,
    f_start_line: Field,
    f_signature: Field,
    f_doc: Field,
    f_language: Field,
}

/// Read-only handle for querying an existing persistent search index.
///
/// Does not acquire a writer lock, so it's safe to use while another
/// process (e.g. `graphy dev`) holds the writer.
pub struct SearchReader {
    index: Index,
    reader: IndexReader,
    f_id: Field,
    f_name: Field,
    f_kind: Field,
    f_file_path: Field,
    f_start_line: Field,
    f_signature: Field,
    f_doc: Field,
    f_language: Field,
}

impl SearchIndex {
    /// Create a new in-memory search index.
    pub fn new_in_memory() -> Result<Self> {
        let schema = Self::build_schema();
        let index = Index::create_in_ram(schema.clone());
        Self::from_index(index, schema, None)
    }

    /// Create a persistent search index at the given path.
    pub fn new_persistent(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        let schema = Self::build_schema();
        let index = Index::create_in_dir(path, schema.clone())
            .or_else(|_| Index::open_in_dir(path))?;
        Self::from_index(index, schema, Some(path.to_path_buf()))
    }

    fn build_schema() -> Schema {
        let mut builder = Schema::builder();
        builder.add_u64_field("id", INDEXED | STORED | FAST);
        builder.add_text_field("name", TEXT | STORED);
        builder.add_text_field("kind", STRING | STORED);
        builder.add_text_field("file_path", STRING | STORED);
        builder.add_u64_field("start_line", STORED | FAST);
        builder.add_text_field("signature", TEXT | STORED);
        builder.add_text_field("doc", TEXT | STORED);
        builder.add_text_field("language", STRING | STORED);
        builder.build()
    }

    fn from_index(index: Index, schema: Schema, persist_path: Option<PathBuf>) -> Result<Self> {
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let fields = Self::resolve_fields(&schema);
        Ok(Self {
            write_lock: Mutex::new(()),
            persist_path,
            reader,
            index,
            schema,
            f_id: fields.0,
            f_name: fields.1,
            f_kind: fields.2,
            f_file_path: fields.3,
            f_start_line: fields.4,
            f_signature: fields.5,
            f_doc: fields.6,
            f_language: fields.7,
        })
    }

    fn resolve_fields(schema: &Schema) -> (Field, Field, Field, Field, Field, Field, Field, Field) {
        (
            schema.get_field("id").expect("schema missing 'id'"),
            schema.get_field("name").expect("schema missing 'name'"),
            schema.get_field("kind").expect("schema missing 'kind'"),
            schema.get_field("file_path").expect("schema missing 'file_path'"),
            schema.get_field("start_line").expect("schema missing 'start_line'"),
            schema.get_field("signature").expect("schema missing 'signature'"),
            schema.get_field("doc").expect("schema missing 'doc'"),
            schema.get_field("language").expect("schema missing 'language'"),
        )
    }

    /// Full rebuild: index all nodes from a CodeGraph.
    ///
    /// For persistent indexes, uses `SingleSegmentIndexWriter` which creates
    /// zero background threads (vs. `IndexWriter`'s 6 threads).
    /// For in-memory indexes, uses a short-lived `IndexWriter`.
    pub fn index_graph(&self, graph: &CodeGraph) -> Result<()> {
        let _guard = self.write_lock.lock().map_err(|e| anyhow::anyhow!("write lock poisoned: {e}"))?;

        if let Some(dir) = &self.persist_path {
            self.index_graph_single_segment(graph, dir)
        } else {
            self.index_graph_with_writer(graph)
        }
    }

    /// Persistent path: SingleSegmentIndexWriter (zero thread overhead).
    fn index_graph_single_segment(&self, graph: &CodeGraph, dir: &Path) -> Result<()> {
        let index = Index::create_in_dir(dir, self.schema.clone())
            .unwrap_or_else(|_| self.index.clone());

        let mut writer: SingleSegmentIndexWriter<TantivyDocument> =
            SingleSegmentIndexWriter::new(index, 15_000_000)?;

        let mut count = 0u64;
        for node in graph.all_nodes() {
            if matches!(node.kind, NodeKind::Folder | NodeKind::Parameter) {
                continue;
            }
            writer.add_document(doc!(
                self.f_id => node.id.as_u64(),
                self.f_name => node.name.as_str(),
                self.f_kind => format!("{:?}", node.kind),
                self.f_file_path => node.file_path.to_string_lossy().as_ref(),
                self.f_start_line => node.span.start_line as u64,
                self.f_signature => node.signature.as_deref().unwrap_or(""),
                self.f_doc => node.doc.as_deref().unwrap_or(""),
                self.f_language => format!("{:?}", node.language),
            ))?;
            count += 1;
        }

        writer.finalize()?;
        self.reader.reload()?;
        info!("Indexed {count} symbols in search");
        Ok(())
    }

    /// In-memory / fallback: short-lived IndexWriter (threads created and dropped).
    fn index_graph_with_writer(&self, graph: &CodeGraph) -> Result<()> {
        let mut writer: IndexWriter = self.index.writer_with_num_threads(1, 15_000_000)?;

        writer.delete_all_documents()?;

        let mut count = 0u64;
        for node in graph.all_nodes() {
            if matches!(node.kind, NodeKind::Folder | NodeKind::Parameter) {
                continue;
            }
            writer.add_document(doc!(
                self.f_id => node.id.as_u64(),
                self.f_name => node.name.as_str(),
                self.f_kind => format!("{:?}", node.kind),
                self.f_file_path => node.file_path.to_string_lossy().as_ref(),
                self.f_start_line => node.span.start_line as u64,
                self.f_signature => node.signature.as_deref().unwrap_or(""),
                self.f_doc => node.doc.as_deref().unwrap_or(""),
                self.f_language => format!("{:?}", node.language),
            ))?;
            count += 1;
        }

        writer.commit()?;
        drop(writer);
        self.reader.reload()?;
        info!("Indexed {count} symbols in search");
        Ok(())
    }

    /// Incremental update: re-index only the symbols belonging to changed files.
    ///
    /// Uses a short-lived `IndexWriter` (creates 6 threads, drops them after commit).
    pub fn update_files(&self, graph: &CodeGraph, changed_files: &[PathBuf]) -> Result<()> {
        let _guard = self.write_lock.lock().map_err(|e| anyhow::anyhow!("write lock poisoned: {e}"))?;
        let mut writer: IndexWriter = self.index.writer_with_num_threads(1, 15_000_000)?;

        // Delete documents for changed/deleted files
        for path in changed_files {
            let path_str = path.to_string_lossy();
            let term = tantivy::Term::from_field_text(self.f_file_path, &path_str);
            writer.delete_term(term);
        }

        // Re-add current nodes for those files (single pass over all nodes)
        let changed_set: std::collections::HashSet<&PathBuf> = changed_files.iter().collect();
        let mut count = 0u64;
        for node in graph.all_nodes() {
            if !changed_set.contains(&node.file_path) {
                continue;
            }
            if matches!(node.kind, NodeKind::Folder | NodeKind::Parameter) {
                continue;
            }
            writer.add_document(doc!(
                self.f_id => node.id.as_u64(),
                self.f_name => node.name.as_str(),
                self.f_kind => format!("{:?}", node.kind),
                self.f_file_path => node.file_path.to_string_lossy().as_ref(),
                self.f_start_line => node.span.start_line as u64,
                self.f_signature => node.signature.as_deref().unwrap_or(""),
                self.f_doc => node.doc.as_deref().unwrap_or(""),
                self.f_language => format!("{:?}", node.language),
            ))?;
            count += 1;
        }

        writer.commit()?;
        // Writer dropped here — tantivy threads will eventually clean up.
        drop(writer);
        self.reader.reload()?;
        info!("Incrementally updated search: {count} symbols across {} files", changed_files.len());
        Ok(())
    }

    /// Hybrid search: BM25 text search with fuzzy fallback.
    pub fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();
        let text_query = self.build_text_query(query);

        let top_docs = searcher.search(&*text_query, &TopDocs::with_limit(max_results))?;
        self.collect_results(&searcher, &top_docs)
    }

    /// Search with optional kind, language, and file path filters.
    pub fn search_filtered(
        &self,
        query: &str,
        kind: Option<&str>,
        language: Option<&str>,
        file: Option<&str>,
        max_results: usize,
    ) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();

        let has_filters = kind.is_some() || language.is_some() || file.is_some();
        let has_text = !query.is_empty();

        if !has_filters && has_text {
            let text_query = self.build_text_query(query);
            let top_docs = searcher.search(&*text_query, &TopDocs::with_limit(max_results))?;
            return self.collect_results(&searcher, &top_docs);
        }

        let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();

        if has_text {
            clauses.push((Occur::Must, self.build_text_query(query)));
        }

        if let Some(k) = kind {
            let term = tantivy::Term::from_field_text(self.f_kind, k);
            clauses.push((
                Occur::Must,
                Box::new(tantivy::query::TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }

        if let Some(l) = language {
            let term = tantivy::Term::from_field_text(self.f_language, l);
            clauses.push((
                Occur::Must,
                Box::new(tantivy::query::TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }

        if let Some(f) = file {
            let combined = if clauses.is_empty() {
                Box::new(tantivy::query::AllQuery) as Box<dyn tantivy::query::Query>
            } else {
                Box::new(BooleanQuery::new(clauses))
            };
            let top_docs = searcher.search(&*combined, &TopDocs::with_limit(max_results * 10))?;
            let all = self.collect_results(&searcher, &top_docs)?;
            let lower = f.to_lowercase();
            return Ok(all.into_iter()
                .filter(|r| r.file_path.to_lowercase().contains(&lower))
                .take(max_results)
                .collect());
        }

        if clauses.is_empty() {
            return Ok(vec![]);
        }

        let combined = BooleanQuery::new(clauses);
        let top_docs = searcher.search(&combined, &TopDocs::with_limit(max_results))?;
        self.collect_results(&searcher, &top_docs)
    }

    /// Search filtered by node kind.
    pub fn search_by_kind(
        &self,
        query: &str,
        kind: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_filtered(query, Some(kind), None, None, max_results)
    }

    /// Build a text query: BM25 parse with fuzzy fallback on empty results.
    fn build_text_query(&self, query: &str) -> Box<dyn tantivy::query::Query> {
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_name, self.f_signature, self.f_doc],
        );
        match query_parser.parse_query(query) {
            Ok(q) => {
                let searcher = self.reader.searcher();
                let probe = searcher.search(&*q, &TopDocs::with_limit(1));
                if probe.map_or(false, |hits| hits.is_empty()) {
                    if !query.contains(' ') {
                        let term = tantivy::Term::from_field_text(self.f_name, query);
                        return Box::new(FuzzyTermQuery::new(term, 2, true));
                    }
                }
                q
            }
            Err(_) => {
                let term = tantivy::Term::from_field_text(self.f_name, query);
                Box::new(FuzzyTermQuery::new(term, 2, true))
            }
        }
    }

    fn collect_results(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: &[(f32, tantivy::DocAddress)],
    ) -> Result<Vec<SearchResult>> {
        collect_results_from(searcher, top_docs, self.f_id, self.f_name, self.f_kind,
            self.f_file_path, self.f_start_line, self.f_signature, self.f_doc, self.f_language)
    }
}

// ── Read-only search handle ─────────────────────────────────────

impl SearchReader {
    /// Open an existing persistent index for reading only.
    /// Does not create a writer — safe to use while another process writes.
    pub fn open(path: &Path) -> Result<Self> {
        let index = Index::open_in_dir(path)?;
        let schema = index.schema();
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let fields = SearchIndex::resolve_fields(&schema);
        Ok(Self {
            index,
            reader,
            f_id: fields.0,
            f_name: fields.1,
            f_kind: fields.2,
            f_file_path: fields.3,
            f_start_line: fields.4,
            f_signature: fields.5,
            f_doc: fields.6,
            f_language: fields.7,
        })
    }

    pub fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();
        let text_query = self.build_text_query(query);

        let top_docs = searcher.search(&*text_query, &TopDocs::with_limit(max_results))?;
        collect_results_from(&searcher, &top_docs, self.f_id, self.f_name, self.f_kind,
            self.f_file_path, self.f_start_line, self.f_signature, self.f_doc, self.f_language)
    }

    pub fn search_by_kind(&self, query: &str, kind: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();
        let text_query = self.build_text_query(query);

        let kind_term = tantivy::Term::from_field_text(self.f_kind, kind);
        let kind_query = tantivy::query::TermQuery::new(kind_term, IndexRecordOption::Basic);
        let combined = BooleanQuery::new(vec![
            (Occur::Must, text_query),
            (Occur::Must, Box::new(kind_query)),
        ]);

        let top_docs = searcher.search(&combined, &TopDocs::with_limit(max_results))?;
        collect_results_from(&searcher, &top_docs, self.f_id, self.f_name, self.f_kind,
            self.f_file_path, self.f_start_line, self.f_signature, self.f_doc, self.f_language)
    }

    fn build_text_query(&self, query: &str) -> Box<dyn tantivy::query::Query> {
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_name, self.f_signature, self.f_doc],
        );
        match query_parser.parse_query(query) {
            Ok(q) => {
                let searcher = self.reader.searcher();
                let probe = searcher.search(&*q, &TopDocs::with_limit(1));
                if probe.map_or(false, |hits| hits.is_empty()) && !query.contains(' ') {
                    let term = tantivy::Term::from_field_text(self.f_name, query);
                    return Box::new(FuzzyTermQuery::new(term, 2, true));
                }
                q
            }
            Err(_) => {
                let term = tantivy::Term::from_field_text(self.f_name, query);
                Box::new(FuzzyTermQuery::new(term, 2, true))
            }
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn collect_results_from(
    searcher: &tantivy::Searcher,
    top_docs: &[(f32, tantivy::DocAddress)],
    f_id: Field, f_name: Field, f_kind: Field, f_file_path: Field,
    f_start_line: Field, f_signature: Field, f_doc: Field, f_language: Field,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::with_capacity(top_docs.len());

    for &(score, doc_address) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_address)?;

        let id = doc.get_first(f_id).and_then(|v| v.as_u64()).unwrap_or(0);
        let name = doc.get_first(f_name).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = doc.get_first(f_kind).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let file_path = doc.get_first(f_file_path).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let start_line = doc.get_first(f_start_line).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let signature = doc.get_first(f_signature).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
        let doc_str = doc.get_first(f_doc).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
        let language = doc.get_first(f_language).and_then(|v| v.as_str()).unwrap_or("").to_string();

        results.push(SearchResult {
            symbol_id: id,
            name,
            kind,
            file_path,
            start_line,
            signature,
            doc: doc_str,
            language,
            score,
        });
    }

    Ok(results)
}

/// Default search index path for a project.
pub fn default_search_path(project_root: &Path) -> PathBuf {
    project_root.join(".graphy").join("search")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::*;
    use std::path::PathBuf;

    fn make_graph() -> CodeGraph {
        let mut g = CodeGraph::new();
        let mut add_func = |name: &str, kind: NodeKind, line: u32| {
            let mut node = GirNode::new(
                name.to_string(),
                kind,
                PathBuf::from("test.py"),
                Span::new(line, 0, line + 5, 0),
                Language::Python,
            );
            node.signature = Some(format!("def {name}()"));
            node.doc = Some(format!("Documentation for {name}"));
            g.add_node(node);
        };

        add_func("hello_world", NodeKind::Function, 1);
        add_func("process_data", NodeKind::Function, 10);
        add_func("MyClass", NodeKind::Class, 20);
        add_func("calculate_sum", NodeKind::Function, 30);
        g
    }

    #[test]
    fn search_by_name() {
        let graph = make_graph();
        let idx = SearchIndex::new_in_memory().unwrap();
        idx.index_graph(&graph).unwrap();

        let results = idx.search("hello", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "hello_world");
    }

    #[test]
    fn search_by_doc() {
        let graph = make_graph();
        let idx = SearchIndex::new_in_memory().unwrap();
        idx.index_graph(&graph).unwrap();

        let results = idx.search("calculate", 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn search_empty_query() {
        let graph = make_graph();
        let idx = SearchIndex::new_in_memory().unwrap();
        idx.index_graph(&graph).unwrap();

        let results = idx.search("", 10).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn search_no_match() {
        let graph = make_graph();
        let idx = SearchIndex::new_in_memory().unwrap();
        idx.index_graph(&graph).unwrap();

        let results = idx.search("zzz_nonexistent_symbol_xyz", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_by_kind_filter() {
        let graph = make_graph();
        let idx = SearchIndex::new_in_memory().unwrap();
        idx.index_graph(&graph).unwrap();

        let results = idx.search_by_kind("MyClass", "Class", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == "Class"));
    }

    #[test]
    fn search_empty_index() {
        let idx = SearchIndex::new_in_memory().unwrap();
        let graph = CodeGraph::new();
        idx.index_graph(&graph).unwrap();

        let results = idx.search("hello", 10).unwrap();
        assert!(results.is_empty());
    }
}

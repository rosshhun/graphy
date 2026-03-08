//! Vector embedding search using fastembed-rs (BGE-small-en-v1.5).
//!
//! Gated behind the `vectors` feature flag. Generates 384-dim embeddings
//! for symbols and supports semantic similarity search.

use std::path::{Path, PathBuf};

use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::info;

use graphy_core::{CodeGraph, NodeKind, SymbolId};

/// A stored embedding for a single symbol.
#[derive(Serialize, Deserialize)]
struct SymbolEmbedding {
    id: SymbolId,
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    vector: Vec<f32>,
}

/// Vector-based semantic search index.
pub struct VectorIndex {
    model: TextEmbedding,
    embeddings: Vec<SymbolEmbedding>,
}

/// Result from a semantic search query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SemanticResult {
    pub symbol_id: u64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: u32,
    pub similarity: f32,
}

impl VectorIndex {
    /// Create a new vector index, downloading the model on first use.
    pub fn new() -> Result<Self> {
        info!("Loading embedding model BGE-small-en-v1.5...");
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
        )?;
        info!("Embedding model loaded");

        Ok(Self {
            model,
            embeddings: Vec::new(),
        })
    }

    /// Save the embeddings to disk (model is not saved; only the embedding data).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = bincode::serialize(&self.embeddings)?;
        std::fs::write(path, bytes)?;
        info!("Saved {} embeddings to {}", self.embeddings.len(), path.display());
        Ok(())
    }

    /// Load embeddings from disk and re-initialize the embedding model.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let embeddings: Vec<SymbolEmbedding> = bincode::deserialize(&bytes)?;
        info!("Loaded {} embeddings from {}", embeddings.len(), path.display());

        info!("Loading embedding model BGE-small-en-v1.5...");
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
        )?;
        info!("Embedding model loaded");

        Ok(Self { model, embeddings })
    }

    /// Generate embeddings for all searchable symbols in the graph.
    pub fn index_graph(&mut self, graph: &CodeGraph) -> Result<()> {
        self.embeddings.clear();

        // Collect searchable nodes first, then build text representations in parallel
        let nodes: Vec<_> = graph
            .all_nodes()
            .filter(|n| !matches!(n.kind, NodeKind::Folder | NodeKind::Parameter))
            .collect();

        let symbols: Vec<_> = nodes
            .par_iter()
            .map(|n| {
                // Build a text representation combining name, signature, and doc
                let mut text = n.name.clone();
                if let Some(sig) = &n.signature {
                    text.push(' ');
                    text.push_str(sig);
                }
                if let Some(doc) = &n.doc {
                    text.push(' ');
                    text.push_str(doc);
                }

                (
                    n.id,
                    n.name.clone(),
                    format!("{:?}", n.kind),
                    n.file_path.to_string_lossy().into_owned(),
                    n.span.start_line,
                    text,
                )
            })
            .collect();

        if symbols.is_empty() {
            info!("No symbols to embed");
            return Ok(());
        }

        let texts: Vec<String> = symbols.iter().map(|s| s.5.clone()).collect();

        // Embed in batches to manage memory
        let batch_size = 256;
        let mut all_vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(batch_size) {
            let batch_vecs = self.model.embed(chunk.to_vec(), None)?;
            all_vectors.extend(batch_vecs);
        }

        for (i, (id, name, kind, file_path, start_line, _text)) in
            symbols.into_iter().enumerate()
        {
            self.embeddings.push(SymbolEmbedding {
                id,
                name,
                kind,
                file_path,
                start_line,
                vector: all_vectors[i].clone(),
            });
        }

        info!("Generated embeddings for {} symbols", self.embeddings.len());
        Ok(())
    }

    /// Search for symbols semantically similar to the query.
    pub fn search(&mut self, query: &str, max_results: usize) -> Result<Vec<SemanticResult>> {
        if self.embeddings.is_empty() {
            return Ok(Vec::new());
        }

        let query_vectors = self.model.embed(vec![query.to_string()], None)?;
        let query_vec = &query_vectors[0];

        // Compute cosine similarity against all embeddings
        let mut scored: Vec<(usize, f32)> = self
            .embeddings
            .iter()
            .enumerate()
            .map(|(i, emb)| (i, cosine_similarity(query_vec, &emb.vector)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_results);

        let results = scored
            .into_iter()
            .map(|(i, sim)| {
                let emb = &self.embeddings[i];
                SemanticResult {
                    symbol_id: emb.id.as_u64(),
                    name: emb.name.clone(),
                    kind: emb.kind.clone(),
                    file_path: emb.file_path.clone(),
                    start_line: emb.start_line,
                    similarity: sim,
                }
            })
            .collect();

        Ok(results)
    }

    /// Generate SIMILAR_TO edges in the graph for each symbol's top-k neighbors.
    ///
    /// For each symbol with an embedding, finds the `k` most similar other symbols
    /// (above `min_similarity` threshold) and adds `EdgeKind::SimilarTo` edges.
    pub fn add_similarity_edges(
        &self,
        graph: &mut CodeGraph,
        k: usize,
        min_similarity: f32,
    ) -> usize {
        use graphy_core::{EdgeKind, GirEdge};

        let mut edge_count = 0;

        for emb in &self.embeddings {
            let mut scored: Vec<(usize, f32)> = self
                .embeddings
                .iter()
                .enumerate()
                .filter(|(_, other)| other.id != emb.id)
                .map(|(i, other)| (i, cosine_similarity(&emb.vector, &other.vector)))
                .filter(|(_, sim)| *sim >= min_similarity)
                .collect();

            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(k);

            for (i, sim) in scored {
                let target_id = self.embeddings[i].id;
                graph.add_edge(
                    emb.id,
                    target_id,
                    GirEdge::new(EdgeKind::SimilarTo).with_confidence(sim),
                );
                edge_count += 1;
            }
        }

        info!("Added {edge_count} SimilarTo edges (k={k}, threshold={min_similarity})");
        edge_count
    }

    /// Find the top-k most similar symbols to a given symbol.
    pub fn similar_to(&self, symbol_id: SymbolId, k: usize) -> Vec<SemanticResult> {
        let source = match self.embeddings.iter().find(|e| e.id == symbol_id) {
            Some(e) => e,
            None => return Vec::new(),
        };

        let mut scored: Vec<(usize, f32)> = self
            .embeddings
            .iter()
            .enumerate()
            .filter(|(_, e)| e.id != symbol_id)
            .map(|(i, e)| (i, cosine_similarity(&source.vector, &e.vector)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        scored
            .into_iter()
            .map(|(i, sim)| {
                let emb = &self.embeddings[i];
                SemanticResult {
                    symbol_id: emb.id.as_u64(),
                    name: emb.name.clone(),
                    kind: emb.kind.clone(),
                    file_path: emb.file_path.clone(),
                    start_line: emb.start_line,
                    similarity: sim,
                }
            })
            .collect()
    }
}

/// Default path for persisted vector embeddings under a project root.
pub fn default_vector_path(project_root: &Path) -> PathBuf {
    project_root.join(".graphy").join("vectors.bin")
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_empty_vectors() {
        // Empty corpus / zero-dimension vectors should not panic and should return 0.0
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        // dot product is 0.0, norms are 0.0, so the zero-norm guard should return 0.0
        assert_eq!(cosine_similarity(&a, &b), 0.0);

        // One empty, one non-empty (mismatched dimensions) — zip produces nothing
        let c = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &c), 0.0);
        assert_eq!(cosine_similarity(&c, &a), 0.0);
    }
}

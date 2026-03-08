async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${res.statusText}`);
  return res.json();
}

// ── Types ──────────────────────────────────────────────────

export interface Stats {
  nodes: number;
  edges: number;
  files: number;
  classes: number;
  structs: number;
  enums: number;
  traits: number;
  functions: number;
  methods: number;
  imports: number;
  variables: number;
  constants: number;
}

export interface SearchResult {
  symbol_id: number;
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
  signature: string | null;
  doc: string | null;
  language: string;
  score: number;
}

export interface SymbolDetail {
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
  end_line: number;
  visibility: string;
  language: string;
  signature: string | null;
  doc: string | null;
  complexity: ComplexityInfo | null;
  callers: SymbolRef[];
  callees: SymbolRef[];
  children: SymbolRef[];
}

export interface ComplexityInfo {
  cyclomatic: number;
  cognitive: number;
  loc: number;
  sloc: number;
  parameter_count: number;
  max_nesting_depth: number;
}

export interface SymbolRef {
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface GraphNode {
  id: string;
  label: string;
  kind: string;
  file: string;
  size: number;
  visibility: string;
  complexity: number | null;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: string;
  confidence: number;
}

export interface HotspotItem {
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
  cyclomatic: number;
  cognitive: number;
  loc: number;
  caller_count: number;
  risk_score: number;
}

export interface DeadCodeItem {
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
  visibility: string;
  dead_probability: number;
}

export interface TaintPath {
  target_name: string;
  target_file: string;
  target_line: number;
  sources: SymbolRef[];
}

export interface ArchitectureData {
  file_count: number;
  symbol_count: number;
  edge_count: number;
  languages: { language: string; count: number }[];
  largest_files: { path: string; symbol_count: number }[];
  kind_distribution: { kind: string; count: number }[];
  edge_distribution: { kind: string; count: number }[];
}

export interface PatternFinding {
  pattern: string;
  severity: string;
  symbol_name: string;
  detail: string;
  file_path: string;
  line: number;
}

export interface ApiSurface {
  public: ApiSymbolEntry[];
  effectively_internal: ApiSymbolEntry[];
  internal_count: number;
  private_count: number;
}

export interface ApiSymbolEntry {
  name: string;
  kind: string;
  file_path: string;
  start_line: number;
  signature: string | null;
  external_callers: number;
}

export interface FileContent {
  path: string;
  content: string;
  line_count: number;
  size_bytes: number;
}

export interface FileSymbol {
  name: string;
  kind: string;
  start_line: number;
  end_line: number;
  children: FileSymbol[];
}

export interface FileSymbolsResponse {
  path: string;
  symbols: FileSymbol[];
  symbol_count: number;
}

// ── API Functions ──────────────────────────────────────────

export const api = {
  stats: () => fetchJson<Stats>('/api/stats'),
  search: (q: string, limit = 20, opts?: { kind?: string; lang?: string; file?: string }) => {
    let url = `/api/search?q=${encodeURIComponent(q)}&limit=${limit}`;
    if (opts?.kind) url += `&kind=${encodeURIComponent(opts.kind)}`;
    if (opts?.lang) url += `&lang=${encodeURIComponent(opts.lang)}`;
    if (opts?.file) url += `&file=${encodeURIComponent(opts.file)}`;
    return fetchJson<SearchResult[]>(url);
  },
  symbol: (name: string) =>
    fetchJson<SymbolDetail[]>(`/api/symbol/${encodeURIComponent(name)}`),
  graph: () => fetchJson<GraphData>('/api/graph'),
  files: () => fetchJson<string[]>('/api/files'),
  hotspots: (limit = 20) => fetchJson<HotspotItem[]>(`/api/hotspots?limit=${limit}`),
  deadCode: (limit = 20) => fetchJson<DeadCodeItem[]>(`/api/dead-code?limit=${limit}`),
  taint: () => fetchJson<TaintPath[]>('/api/taint'),
  architecture: () => fetchJson<ArchitectureData>('/api/architecture'),
  patterns: (limit = 20) => fetchJson<PatternFinding[]>(`/api/patterns?limit=${limit}`),
  apiSurface: () => fetchJson<ApiSurface>('/api/api-surface'),
  fileContent: (path: string) =>
    fetchJson<FileContent>(`/api/file-content?path=${encodeURIComponent(path)}`),
  fileSymbols: (path: string) =>
    fetchJson<FileSymbolsResponse>(`/api/file-symbols?path=${encodeURIComponent(path)}`),
};

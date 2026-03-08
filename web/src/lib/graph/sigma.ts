import Graph from 'graphology';
import forceAtlas2 from 'graphology-layout-forceatlas2';
import { random as randomLayout } from 'graphology-layout';
import Sigma from 'sigma';
import type { GraphData } from '../api/client.js';
import { KIND_COLORS } from '../stores/app.svelte.js';

// ── Color Utilities ──

function hexToRgb(hex: string): [number, number, number] {
  if (!hex || !hex.startsWith('#') || hex.length < 7) return [107, 114, 128];
  return [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ];
}

function hexToRgba(hex: string, alpha: number): string {
  const [r, g, b] = hexToRgb(hex);
  return `rgba(${r},${g},${b},${alpha})`;
}

// Default edge alpha: barely-there hints, edges come alive on hover/focus
const EDGE_DEFAULT_ALPHA = 0.018;

// Active (hover/focus) edge visibility — much brighter
const EDGE_ACTIVE_ALPHA: Record<string, number> = {
  Calls: 0.55,
  Implements: 0.45,
  Overrides: 0.45,
  Inherits: 0.45,
  ImportsFrom: 0.25,
  Contains: 0.15,
  ParentOf: 0.15,
  ReturnsType: 0.25,
  HasParameter: 0.2,
  AnnotatedWith: 0.25,
  DataFlowsTo: 0.5,
};

// ── Custom hover renderer: soft glow + dark label pill ──

function drawRoundRect(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number, r: number,
) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.arcTo(x + w, y, x + w, y + r, r);
  ctx.lineTo(x + w, y + h - r);
  ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
  ctx.lineTo(x + r, y + h);
  ctx.arcTo(x, y + h, x, y + h - r, r);
  ctx.lineTo(x, y + r);
  ctx.arcTo(x, y, x + r, y, r);
  ctx.closePath();
}

function drawNodeHoverGlow(
  context: CanvasRenderingContext2D,
  data: Record<string, any>,
  settings: Record<string, any>,
): void {
  const x: number = data.x;
  const y: number = data.y;
  const size: number = data.size;
  const label: string | null = data.label;
  const color: string = data.color || '#6b7280';

  const [cr, cg, cb] = hexToRgb(color);

  // ── Outer glow ──
  const glowRadius = size * 3.5;
  const grad = context.createRadialGradient(x, y, size * 0.4, x, y, glowRadius);
  grad.addColorStop(0, `rgba(${cr},${cg},${cb},0.4)`);
  grad.addColorStop(0.3, `rgba(${cr},${cg},${cb},0.15)`);
  grad.addColorStop(0.6, `rgba(${cr},${cg},${cb},0.04)`);
  grad.addColorStop(1, `rgba(${cr},${cg},${cb},0)`);
  context.beginPath();
  context.arc(x, y, glowRadius, 0, Math.PI * 2);
  context.fillStyle = grad;
  context.fill();

  // ── Node circle with subtle bright ring ──
  context.beginPath();
  context.arc(x, y, size + 1.5, 0, Math.PI * 2);
  context.fillStyle = `rgba(${cr},${cg},${cb},0.25)`;
  context.fill();

  context.beginPath();
  context.arc(x, y, size, 0, Math.PI * 2);
  context.fillStyle = color;
  context.fill();

  // ── Label pill ──
  if (label) {
    const fontSize = (settings.labelSize || 12) + 1;
    const fontFamily = settings.labelFont || 'sans-serif';
    context.font = `500 ${fontSize}px ${fontFamily}`;

    const textWidth = context.measureText(label).width;
    const padX = 8;
    const padY = 5;
    const gap = 10;
    const bgX = x + size + gap;
    const bgY = y - fontSize / 2 - padY;
    const bgW = textWidth + padX * 2;
    const bgH = fontSize + padY * 2;

    // Shadow
    context.shadowColor = 'rgba(0,0,0,0.5)';
    context.shadowBlur = 8;
    context.shadowOffsetX = 0;
    context.shadowOffsetY = 2;

    // Dark pill background
    drawRoundRect(context, bgX, bgY, bgW, bgH, 6);
    context.fillStyle = 'rgba(8, 8, 12, 0.94)';
    context.fill();

    // Reset shadow
    context.shadowColor = 'transparent';
    context.shadowBlur = 0;
    context.shadowOffsetX = 0;
    context.shadowOffsetY = 0;

    // Color accent border
    context.strokeStyle = `rgba(${cr},${cg},${cb},0.5)`;
    context.lineWidth = 1;
    context.stroke();

    // Label text
    context.fillStyle = '#f1f5f9';
    context.textBaseline = 'middle';
    context.fillText(label, bgX + padX, y + 0.5);
  }
}

// ── Types ──

export interface GraphController {
  sigma: Sigma;
  setFocusedFile(file: string | null): void;
  zoomIn(): void;
  zoomOut(): void;
  resetZoom(): void;
  destroy(): void;
}

// ── Graph Creation ──

export function createGraph(data: GraphData): Graph {
  const graph = new Graph();

  // Pre-compute degree for sizing
  const degreeMap = new Map<string, number>();
  for (const edge of data.edges) {
    degreeMap.set(edge.source, (degreeMap.get(edge.source) || 0) + 1);
    degreeMap.set(edge.target, (degreeMap.get(edge.target) || 0) + 1);
  }

  for (const node of data.nodes) {
    const degree = degreeMap.get(node.id) || 0;
    const isFile = node.kind === 'File';

    // Degree-weighted sizing: important nodes stand out
    let size: number;
    if (isFile) {
      size = 2.5; // Files are always small background dots
    } else {
      // Base 4, scale with log of degree, cap at 22
      size = Math.min(22, 4 + Math.log2(1 + degree) * 3.5 + (node.size - 4) * 0.5);
      size = Math.max(3.5, size);
    }

    graph.addNode(node.id, {
      label: isFile ? '' : node.label, // Files don't get labels
      size,
      color: KIND_COLORS[node.kind] || '#6b7280',
      kind: node.kind,
      file: node.file,
      visibility: node.visibility,
      complexity: node.complexity,
      _origLabel: node.label,
    });
  }

  for (const edge of data.edges) {
    if (graph.hasNode(edge.source) && graph.hasNode(edge.target)) {
      try {
        const alpha = EDGE_DEFAULT_ALPHA;
        graph.addEdge(edge.source, edge.target, {
          kind: edge.kind,
          size: 0.2,
          color: `rgba(140,145,155,${alpha})`,
        });
      } catch {
        // Skip duplicate edges
      }
    }
  }

  return graph;
}

// ── Layout ──

export function runForceLayout(graph: Graph): void {
  if (graph.order === 0) return;

  // Collect isolated nodes (degree 0) — FA2 can't position them meaningfully
  const isolatedNodes = new Set<string>();
  const connectedNodes = new Set<string>();
  graph.forEachEdge((_edge, _attrs, source, target) => {
    connectedNodes.add(source);
    connectedNodes.add(target);
  });
  graph.forEachNode((node) => {
    if (!connectedNodes.has(node)) isolatedNodes.add(node);
  });

  // Random initial positions
  randomLayout.assign(graph);

  const nodeCount = graph.order;

  // Organic force-directed layout — NO linLogMode (it creates rings)
  forceAtlas2.assign(graph, {
    iterations: nodeCount > 500 ? 120 : 250,
    settings: {
      gravity: 0.4,
      scalingRatio: 80,
      barnesHutOptimize: nodeCount > 80,
      barnesHutTheta: 0.5,
      strongGravityMode: false,
      slowDown: 2,
      adjustSizes: true,
      linLogMode: false,
      outboundAttractionDistribution: false,
    },
  });

  // Compute bounding box of connected nodes to scatter isolates organically
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  graph.forEachNode((node) => {
    if (isolatedNodes.has(node)) return;
    const x = graph.getNodeAttribute(node, 'x') as number;
    const y = graph.getNodeAttribute(node, 'y') as number;
    minX = Math.min(minX, x); maxX = Math.max(maxX, x);
    minY = Math.min(minY, y); maxY = Math.max(maxY, y);
  });

  // Place isolated nodes organically within the graph's extent (not in a ring)
  const rangeX = (maxX - minX) || 100;
  const rangeY = (maxY - minY) || 100;
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const pad = 0.3; // 30% extra around the connected region

  for (const node of isolatedNodes) {
    // Scatter within the graph's footprint with some padding
    const x = cx + (Math.random() - 0.5) * rangeX * (1 + pad);
    const y = cy + (Math.random() - 0.5) * rangeY * (1 + pad);
    graph.setNodeAttribute(node, 'x', x);
    graph.setNodeAttribute(node, 'y', y);
  }

  // Scale up for breathing room
  const scale = 2.5;
  graph.forEachNode((node) => {
    graph.setNodeAttribute(node, 'x', (graph.getNodeAttribute(node, 'x') as number) * scale);
    graph.setNodeAttribute(node, 'y', (graph.getNodeAttribute(node, 'y') as number) * scale);
  });

  // Noverlap: push apart overlapping nodes
  const allNodes = graph.nodes();
  const margin = 16;
  for (let pass = 0; pass < 60; pass++) {
    let moved = false;
    for (let i = 0; i < allNodes.length; i++) {
      const a = allNodes[i];
      const ax = graph.getNodeAttribute(a, 'x') as number;
      const ay = graph.getNodeAttribute(a, 'y') as number;
      const aSize = graph.getNodeAttribute(a, 'size') as number;
      for (let j = i + 1; j < allNodes.length; j++) {
        const b = allNodes[j];
        const bx = graph.getNodeAttribute(b, 'x') as number;
        const by = graph.getNodeAttribute(b, 'y') as number;
        const bSize = graph.getNodeAttribute(b, 'size') as number;
        const dx = bx - ax;
        const dy = by - ay;
        const dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
        const minDist = aSize + bSize + margin;
        if (dist < minDist) {
          const push = (minDist - dist) * 0.5;
          const nx = dx / dist;
          const ny = dy / dist;
          graph.setNodeAttribute(a, 'x', ax - nx * push);
          graph.setNodeAttribute(a, 'y', ay - ny * push);
          graph.setNodeAttribute(b, 'x', bx + nx * push);
          graph.setNodeAttribute(b, 'y', by + ny * push);
          moved = true;
        }
      }
    }
    if (!moved) break;
  }
}

// ── Sigma Renderer ──

export function initSigma(
  container: HTMLElement,
  graph: Graph,
  onClickNode?: (id: string, label: string) => void,
  onFocusCleared?: () => void,
): GraphController {
  // ── Internal state ──
  let hoveredNode: string | null = null;
  let hoveredNeighbors = new Set<string>();
  let hoveredEdges = new Set<string>();

  let focusedFile: string | null = null;
  let focusedNodes = new Set<string>();
  let focusedNeighbors = new Set<string>();
  let focusedEdges = new Set<string>();

  // Is any selection active? (hover or focus)
  function hasSelection(): boolean {
    return hoveredNode !== null || focusedFile !== null;
  }

  // Determine a node's role in the current selection
  function nodeRole(node: string): 'active' | 'neighbor' | 'background' | 'none' {
    if (!hasSelection()) return 'none';

    // Hover takes priority for the hovered node itself
    if (hoveredNode === node) return 'active';

    if (focusedFile) {
      if (focusedNodes.has(node)) return 'active';
      if (focusedNeighbors.has(node)) return 'neighbor';
      // During focus, hover neighbors are also neighbors
      if (hoveredNode && hoveredNeighbors.has(node)) return 'neighbor';
      return 'background';
    }

    // Hover only (no focus)
    if (hoveredNeighbors.has(node)) return 'neighbor';
    return 'background';
  }

  // Determine an edge's role
  function edgeRole(edge: string): 'active' | 'background' | 'none' {
    if (!hasSelection()) return 'none';

    if (hoveredNode && hoveredEdges.has(edge)) return 'active';
    if (focusedFile && focusedEdges.has(edge)) return 'active';
    if (hasSelection()) return 'background';
    return 'none';
  }

  const renderer = new Sigma(graph, container, {
    renderEdgeLabels: false,
    defaultEdgeType: 'arrow',
    defaultNodeColor: '#6b7280',
    labelColor: { color: '#c8d0da' },
    labelFont: "'IBM Plex Mono', 'JetBrains Mono', monospace",
    labelSize: 12,
    labelWeight: '400',
    stagePadding: 50,
    zIndex: true,
    labelRenderedSizeThreshold: 14,
    labelDensity: 0.4,
    labelGridCellSize: 220,

    hoverRenderer: drawNodeHoverGlow as any,

    nodeReducer(node, data) {
      const res = { ...data };
      const role = nodeRole(node);

      switch (role) {
        case 'active':
          res.size = (data.size || 4) * 1.4;
          res.zIndex = 3;
          res.forceLabel = true;
          // Restore label for File nodes during active state
          if (!res.label && data._origLabel) res.label = data._origLabel;
          break;

        case 'neighbor':
          res.color = hexToRgba(data.color || '#6b7280', 0.65);
          res.size = (data.size || 4) * 1.05;
          res.zIndex = 2;
          res.forceLabel = true;
          if (!res.label && data._origLabel) res.label = data._origLabel;
          break;

        case 'background':
          res.color = hexToRgba(data.color || '#6b7280', 0.08);
          res.label = '';
          res.size = (data.size || 4) * 0.4;
          res.zIndex = 0;
          break;

        case 'none':
          // Default state — no selection active
          break;
      }

      return res;
    },

    edgeReducer(edge, data) {
      const res = { ...data };
      const role = edgeRole(edge);

      switch (role) {
        case 'active': {
          const source = graph.source(edge);
          const sourceKind = graph.getNodeAttribute(source, 'kind');
          const baseColor = KIND_COLORS[sourceKind] || '#6b7280';
          const alpha = EDGE_ACTIVE_ALPHA[data.kind] || 0.35;
          res.color = hexToRgba(baseColor, alpha);
          res.size = Math.max(1.2, (data.size || 0.5) * 2.5);
          res.zIndex = 1;
          break;
        }

        case 'background':
          res.color = 'rgba(30,30,30,0.015)';
          res.size = 0.15;
          res.zIndex = 0;
          break;

        case 'none':
          // Default edge rendering from graph creation
          break;
      }

      return res;
    },
  });

  // ── Hover events ──
  renderer.on('enterNode', ({ node }) => {
    hoveredNode = node;
    hoveredNeighbors = new Set(graph.neighbors(node));
    hoveredEdges = new Set(graph.edges(node));
    container.style.cursor = 'pointer';
    renderer.refresh();
  });

  renderer.on('leaveNode', () => {
    hoveredNode = null;
    hoveredNeighbors = new Set();
    hoveredEdges = new Set();
    container.style.cursor = 'default';
    renderer.refresh();
  });

  // ── Click events ──
  if (onClickNode) {
    renderer.on('clickNode', ({ node }) => {
      const label = graph.getNodeAttribute(node, '_origLabel') || graph.getNodeAttribute(node, 'label');
      onClickNode(node, label);
    });
  }

  renderer.on('clickStage', () => {
    if (focusedFile) {
      setFocusedFile(null);
      onFocusCleared?.();
    }
  });

  // ── Focus control ──
  function setFocusedFile(file: string | null) {
    if (file === focusedFile) return;
    focusedFile = file;

    if (file) {
      focusedNodes = new Set();
      focusedNeighbors = new Set();
      focusedEdges = new Set();

      graph.forEachNode((node, attrs) => {
        if (attrs.file === file) focusedNodes.add(node);
      });

      graph.forEachEdge((edge, _attrs, source, target) => {
        const sIn = focusedNodes.has(source);
        const tIn = focusedNodes.has(target);
        if (sIn || tIn) {
          focusedEdges.add(edge);
          if (sIn && !tIn) focusedNeighbors.add(target);
          if (tIn && !sIn) focusedNeighbors.add(source);
        }
      });
    } else {
      focusedNodes = new Set();
      focusedNeighbors = new Set();
      focusedEdges = new Set();
    }

    renderer.refresh();
  }

  return {
    sigma: renderer,
    setFocusedFile,
    zoomIn() { renderer.getCamera().animatedZoom({ duration: 200 }); },
    zoomOut() { renderer.getCamera().animatedUnzoom({ duration: 200 }); },
    resetZoom() { renderer.getCamera().animatedReset({ duration: 200 }); },
    destroy() { renderer.kill(); },
  };
}

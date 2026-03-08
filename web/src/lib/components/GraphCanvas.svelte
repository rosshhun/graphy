<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { GraphData } from '../api/client.js';
  import { createGraph, runForceLayout, initSigma, type GraphController } from '../graph/sigma.js';
  import { app } from '../stores/app.svelte.js';

  let { data, onNodeClick, focusedFile = null, onFocusCleared }: {
    data: GraphData | null;
    onNodeClick: (label: string) => void;
    focusedFile?: string | null;
    onFocusCleared?: () => void;
  } = $props();

  let container = $state<HTMLDivElement | undefined>(undefined);
  let controller = $state<GraphController | null>(null);

  function renderGraph() {
    if (!data || !container) return;
    if (container.clientWidth === 0 || container.clientHeight === 0) return;

    if (controller) {
      controller.destroy();
      controller = null;
    }

    const graph = createGraph(data);
    runForceLayout(graph);
    controller = initSigma(container, graph, (_id, label) => {
      onNodeClick(label);
    }, () => {
      onFocusCleared?.();
    });
  }

  $effect(() => {
    if (data && container) {
      requestAnimationFrame(() => renderGraph());
    }
  });

  // Apply focus when controller or focusedFile changes
  $effect(() => {
    if (controller) {
      controller.setFocusedFile(focusedFile ?? null);
    }
  });

  // Register zoom functions
  $effect(() => {
    if (controller) {
      app.zoomIn = () => controller?.zoomIn();
      app.zoomOut = () => controller?.zoomOut();
      app.zoomReset = () => controller?.resetZoom();
    }
    return () => {
      app.zoomIn = null;
      app.zoomOut = null;
      app.zoomReset = null;
    };
  });

  onDestroy(() => {
    if (controller) controller.destroy();
  });
</script>

<div class="graph-wrapper">
  <div class="graph-container" bind:this={container}></div>
</div>

<style>
  .graph-wrapper {
    width: 100%;
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
  .graph-container {
    width: 100%;
    height: 100%;
    background:
      radial-gradient(circle at 50% 50%, rgba(7, 119, 255, 0.015) 0%, transparent 70%),
      radial-gradient(circle, rgba(255, 255, 255, 0.018) 1px, transparent 1px),
      var(--color-base, #0a0a0a);
    background-size: 100% 100%, 24px 24px, 100% 100%;
  }
</style>

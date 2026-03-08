<script lang="ts">
  import { KIND_COLORS } from '../stores/app.svelte.js';
  import { kindIcon } from '../icons.js';

  let { kind, label }: { kind: string; label?: string } = $props();
  let color = $derived(KIND_COLORS[kind] || '#6b7280');
</script>

<span
  class="badge"
  style="background: {color}1a; color: {color}; border-color: {color}33"
>
  <span class="badge-icon">{@html kindIcon(kind, 13)}</span>
  {label ?? kind}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 7px 2px 5px;
    border-radius: 4px;
    border: 1px solid;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.2px;
    font-family: var(--font-display);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .badge-icon {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
</style>

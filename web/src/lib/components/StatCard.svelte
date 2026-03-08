<script lang="ts">
  let { label, value, color }: { label: string; value: string | number; color?: string } = $props();
</script>

<div class="stat-card" style="--accent: {color || '#58a6ff'}">
  <div class="stat-value">{value}</div>
  <div class="stat-label">{label}</div>
</div>

<style>
  .stat-card {
    background: var(--color-surface, #151b23);
    border: 1px solid var(--color-border, #252d38);
    border-top: 2px solid var(--accent);
    border-radius: 3px;
    padding: 16px 18px;
    transition: border-color 0.2s, transform 0.15s;
    position: relative;
    overflow: hidden;
  }
  .stat-card::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 48px;
    background: linear-gradient(180deg, var(--accent), transparent);
    opacity: 0.05;
    pointer-events: none;
  }
  .stat-card:hover {
    border-color: var(--color-border-bright, #3d4a5c);
    transform: translateY(-1px);
  }
  .stat-value {
    font-size: 24px;
    font-weight: 700;
    font-family: var(--font-display);
    color: var(--accent);
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .stat-label {
    font-size: 10px;
    color: var(--color-text-muted, #4a5568);
    margin-top: 6px;
    text-transform: uppercase;
    letter-spacing: 0.8px;
    font-family: var(--font-mono);
  }
</style>

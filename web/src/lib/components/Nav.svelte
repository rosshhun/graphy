<script lang="ts">
  import { app, type View } from '../stores/app.svelte.js';

  const navItems: { id: View; label: string }[] = [
    { id: 'explorer', label: 'Explorer' },
    { id: 'analysis', label: 'Analysis' },
    { id: 'security', label: 'Security' },
    { id: 'architecture', label: 'Architecture' },
  ];

  function handleNav(id: View) {
    app.activeView = id;
    app.searchActive = false;
  }
</script>

<nav class="sidebar-nav">
  <div class="nav-top">
    <!-- Brand -->
    <button class="nav-brand" onclick={() => handleNav('explorer')} title="Graphy">
      <div class="brand-mark">G</div>
    </button>

    <!-- Search -->
    <button
      class="nav-icon-btn"
      class:active={app.searchActive}
      onclick={() => { app.searchActive = !app.searchActive; }}
      title="Search"
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
      </svg>
    </button>

    <!-- View icons -->
    {#each navItems as item}
      <button
        class="nav-icon-btn"
        class:active={app.activeView === item.id && !app.searchActive}
        onclick={() => handleNav(item.id)}
        title={item.label}
      >
        {#if item.id === 'explorer'}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/>
            <rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>
          </svg>
        {:else if item.id === 'analysis'}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M21 21H4.6c-.56 0-.84 0-1.054-.109a1 1 0 01-.437-.437C3 20.24 3 19.96 3 19.4V3"/>
            <path d="M7 14l4-4 4 4 6-6"/>
          </svg>
        {:else if item.id === 'security'}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
          </svg>
        {:else if item.id === 'architecture'}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <circle cx="12" cy="5" r="3"/><circle cx="5" cy="19" r="3"/><circle cx="19" cy="19" r="3"/>
            <line x1="12" y1="8" x2="5" y2="16"/><line x1="12" y1="8" x2="19" y2="16"/>
          </svg>
        {/if}
      </button>
    {/each}
  </div>

</nav>

<style>
  .sidebar-nav {
    width: 52px;
    height: 100vh;
    background: var(--color-bg);
    display: flex;
    flex-direction: column;
    align-items: center;
    flex-shrink: 0;
    z-index: 100;
    padding: 8px 0;
  }

  .nav-top {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    flex: 1;
  }

  .nav-brand {
    width: 36px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    cursor: pointer;
    margin-bottom: 8px;
    padding: 0;
  }

  .brand-mark {
    width: 30px;
    height: 30px;
    background: linear-gradient(135deg, #0777ff, #6C3FE0);
    color: #fff;
    border-radius: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 16px;
    line-height: 1;
    transition: transform 0.15s, box-shadow 0.15s;
  }

  .nav-brand:hover .brand-mark {
    transform: scale(1.05);
    box-shadow: 0 0 14px rgba(7, 119, 255, 0.35);
  }

  .nav-icon-btn {
    width: 36px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 0.15s, background 0.15s;
    position: relative;
    padding: 0;
  }

  .nav-icon-btn svg {
    width: 20px;
    height: 20px;
  }

  .nav-icon-btn:hover {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  .nav-icon-btn.active {
    color: var(--color-text-primary);
    background: var(--color-surface);
  }

</style>

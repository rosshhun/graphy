<div class="loading">
  <div class="pulse"></div>
  <div class="pulse delay-1"></div>
  <div class="pulse delay-2"></div>
</div>

<style>
  .loading {
    display: flex;
    gap: 6px;
    align-items: center;
    justify-content: center;
    padding: 40px;
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .delay-1 { animation-delay: 0.2s; }
  .delay-2 { animation-delay: 0.4s; }
  @keyframes pulse {
    0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }
</style>

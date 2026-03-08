<script lang="ts">
  import { fileIcon, folderIcon } from '../icons.js';

  let { files, onSelect }: { files: string[]; onSelect: (file: string) => void } = $props();

  interface TreeNode {
    name: string;
    path: string;
    children: TreeNode[];
    isFile: boolean;
    expanded: boolean;
  }

  let tree = $state<TreeNode[]>([]);

  $effect(() => {
    const root: TreeNode = { name: '', path: '', children: [], isFile: false, expanded: true };

    for (const file of files) {
      const parts = file.split('/');
      let current = root;

      for (let i = 0; i < parts.length; i++) {
        const part = parts[i];
        const isLast = i === parts.length - 1;
        let child = current.children.find(c => c.name === part);

        if (!child) {
          child = {
            name: part,
            path: parts.slice(0, i + 1).join('/'),
            children: [],
            isFile: isLast,
            expanded: i < 2,
          };
          current.children.push(child);
        }
        current = child;
      }
    }

    function sortTree(node: TreeNode) {
      node.children.sort((a, b) => {
        if (a.isFile !== b.isFile) return a.isFile ? 1 : -1;
        return a.name.localeCompare(b.name);
      });
      node.children.forEach(sortTree);
    }
    sortTree(root);
    tree = root.children;
  });

  function toggle(node: TreeNode) {
    node.expanded = !node.expanded;
  }
</script>

<div class="file-tree">
  {#each tree as node}
    {@render treeNode(node, 0)}
  {/each}
</div>

{#snippet treeNode(node: TreeNode, depth: number)}
  <button
    class="tree-item"
    class:is-file={node.isFile}
    onclick={() => node.isFile ? onSelect(node.path) : toggle(node)}
  >
    {#if !node.isFile}
      <span class="chevron" class:open={node.expanded}>
        <svg width="9" height="9" viewBox="0 0 9 9">
          <path d="M2.5 1.5L6.5 4.5L2.5 7.5" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </span>
      <span class="node-icon">{@html folderIcon(node.name, node.expanded, 15)}</span>
    {:else}
      <span class="file-spacer"></span>
      <span class="node-icon">{@html fileIcon(node.name, 15)}</span>
    {/if}
    <span class="tree-name">{node.name}</span>
  </button>

  {#if !node.isFile && node.expanded && node.children.length > 0}
    <div class="tree-children">
      {#each node.children as child}
        {@render treeNode(child, depth + 1)}
      {/each}
    </div>
  {/if}
{/snippet}

<style>
  .file-tree {
    overflow-y: auto;
    font-family: var(--font-mono);
    font-size: 13px;
    padding: 4px 0;
    flex: 1;
  }
  .tree-item {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 2px 10px 2px 2px;
    background: none;
    border: none;
    color: var(--color-text-secondary, #737373);
    cursor: pointer;
    text-align: left;
    transition: background 0.08s, color 0.08s;
    white-space: nowrap;
    overflow: hidden;
    font-family: inherit;
    font-size: inherit;
    height: 26px;
  }
  .tree-item:hover {
    background: var(--color-surface-hover, #1a1a1a);
    color: var(--color-text-primary, #e0e0e0);
  }
  .tree-item.is-file {
    color: var(--color-text-primary, #e0e0e0);
  }
  .chevron {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    color: var(--color-text-muted, #404040);
    transition: transform 0.12s ease;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .tree-item:hover .chevron {
    color: var(--color-text-secondary, #737373);
  }
  .node-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }
  .file-spacer {
    width: 16px;
    flex-shrink: 0;
  }
  .tree-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tree-children {
    margin-left: 8px;
    padding-left: 12px;
    border-left: 1px solid var(--color-border, #1e1e1e);
    position: relative;
  }
  .tree-children:hover {
    border-left-color: var(--color-border-bright, #2e2e2e);
  }
</style>

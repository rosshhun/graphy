import type { Stats, SymbolDetail } from '../api/client.js';

export type View = 'explorer' | 'search' | 'analysis' | 'security' | 'architecture';

export const KIND_COLORS: Record<string, string> = {
  Function: '#5ebbff',
  Class: '#c78dff',
  Method: '#3dffa8',
  Struct: '#ffcc00',
  Trait: '#ff6eb4',
  File: '#666666',
  Enum: '#ff8c42',
  Import: '#8899aa',
  Interface: '#b388ff',
  Constant: '#ff6eb4',
  Variable: '#5ef0f0',
  Constructor: '#3dffa8',
};

export const KIND_ABBREV: Record<string, string> = {
  Function: 'Fn', Class: 'C', Method: 'M', Struct: 'S',
  Trait: 'T', Enum: 'E', Interface: 'If', Import: 'Im',
  Variable: 'Var', Constant: 'K', Constructor: 'Ct',
  File: 'Fi', Field: 'Fd', TypeAlias: 'Ta', Module: 'Mod',
  Decorator: 'D', EnumVariant: 'Ev', Property: 'Pr',
};

export const LANG_COLORS: Record<string, string> = {
  Python: '#3572A5', TypeScript: '#3178C6', JavaScript: '#F1E05A',
  Rust: '#DEA584', Go: '#00ADD8', Java: '#B07219',
  Cpp: '#F34B7D', C: '#555555', CSharp: '#178600',
  Ruby: '#701516', PHP: '#4F5D95', Kotlin: '#A97BFF', Svelte: '#FF3E00',
};

class AppStore {
  activeView = $state<View>('explorer');
  selectedSymbol = $state<SymbolDetail | null>(null);
  stats = $state<Stats | null>(null);
  sidebarOpen = $state(true);
  pendingSymbolName = $state<string | null>(null);
  pendingSearchQuery = $state<string | null>(null);
  searchActive = $state(false);
  zoomIn = $state<(() => void) | null>(null);
  zoomOut = $state<(() => void) | null>(null);
  zoomReset = $state<(() => void) | null>(null);
  explorerMode = $state<'graph' | 'code'>('graph');
}

export const app = new AppStore();

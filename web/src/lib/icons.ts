// Icon utilities using simple-icons (languages) + @vscode/codicons (symbols)
//
// Language icons: SVG brand logos from simple-icons (https://simpleicons.org)
// Symbol kind icons: VS Code codicons (https://microsoft.github.io/vscode-codicons)
// File/folder icons: Custom + codicon-style

import {
  siPython, siTypescript, siJavascript, siRust, siGo, siOpenjdk,
  siCplusplus, siC, siRuby, siPhp, siKotlin, siSvelte,
  siMarkdown, siHtml5, siCss,
} from 'simple-icons';

// ── Language Icons ──

interface SimpleIcon {
  title: string;
  slug: string;
  hex: string;
  path: string;
  svg: string;
}

// Map our language names to simple-icons objects + custom hex overrides
const LANG_MAP: Record<string, { icon: SimpleIcon; hex?: string }> = {
  Python:     { icon: siPython as SimpleIcon },
  TypeScript: { icon: siTypescript as SimpleIcon },
  JavaScript: { icon: siJavascript as SimpleIcon, hex: 'F7DF1E' },
  Rust:       { icon: siRust as SimpleIcon, hex: 'DEA584' },
  Go:         { icon: siGo as SimpleIcon },
  Java:       { icon: siOpenjdk as SimpleIcon, hex: 'ED8B00' },
  Cpp:        { icon: siCplusplus as SimpleIcon },
  C:          { icon: siC as SimpleIcon, hex: '6E6E6E' },
  CSharp:     { icon: siCplusplus as SimpleIcon, hex: '512BD4' }, // C++ shape, C# color
  Ruby:       { icon: siRuby as SimpleIcon },
  PHP:        { icon: siPhp as SimpleIcon },
  Kotlin:     { icon: siKotlin as SimpleIcon },
  Svelte:     { icon: siSvelte as SimpleIcon },
  Markdown:   { icon: siMarkdown as SimpleIcon, hex: '519ABA' },
  HTML:       { icon: siHtml5 as SimpleIcon },
  CSS:        { icon: siCss as SimpleIcon },
};

export function langIcon(lang: string, size = 16): string {
  const entry = LANG_MAP[lang];
  if (!entry) {
    // Fallback: first-letter circle
    const letter = lang.charAt(0).toUpperCase();
    return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10" fill="#4a4a4a"/><text x="12" y="16.5" text-anchor="middle" fill="#fff" font-size="12" font-weight="700" font-family="sans-serif">${letter}</text></svg>`;
  }
  const hex = entry.hex || entry.icon.hex;
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" xmlns="http://www.w3.org/2000/svg" fill="#${hex}"><path d="${entry.icon.path}"/></svg>`;
}

// Get just the hex color for a language
export function langHex(lang: string): string {
  const entry = LANG_MAP[lang];
  if (!entry) return '6b7280';
  return entry.hex || entry.icon.hex;
}


// ── Symbol Kind Icons (VS Code Codicons) ──
// Using SVG path data extracted from @vscode/codicons/src/icons/

const CODICON_PATHS: Record<string, string> = {
  // symbol-method.svg (cube/3D box)
  Function: 'M4.69684 5.04043C4.44303 4.93166 4.14909 5.04923 4.04031 5.30305C3.93153 5.55686 4.04911 5.8508 4.30292 5.95958L7.49988 7.3297V10.5C7.49988 10.7761 7.72374 11 7.99988 11C8.27603 11 8.49988 10.7761 8.49988 10.5V7.3297L11.6968 5.95958C11.9507 5.8508 12.0682 5.55686 11.9595 5.30305C11.8507 5.04923 11.5567 4.93166 11.3029 5.04043L7.99988 6.45602L4.69684 5.04043ZM9.07694 1.37855C8.38373 1.11193 7.61627 1.11193 6.92306 1.37855L1.96153 3.28683C1.38224 3.50964 1 4.06619 1 4.68685V11.3133C1 11.9339 1.38224 12.4905 1.96153 12.7133L6.92306 14.6216C7.61627 14.8882 8.38373 14.8882 9.07694 14.6216L14.0385 12.7133C14.6178 12.4905 15 11.9339 15 11.3133V4.68685C15 4.06619 14.6178 3.50964 14.0385 3.28683L9.07694 1.37855ZM7.28204 2.3119C7.74418 2.13415 8.25582 2.13415 8.71796 2.3119L13.6795 4.22018C13.8726 4.29445 14 4.47997 14 4.68685V11.3133C14 11.5201 13.8726 11.7057 13.6795 11.7799L8.71796 13.6882C8.25582 13.866 7.74418 13.866 7.28204 13.6882L2.32051 11.7799C2.12741 11.7057 2 11.5201 2 11.3133V4.68685C2 4.47997 2.12741 4.29445 2.32051 4.22018L7.28204 2.3119Z',

  // symbol-class.svg (connected nodes)
  Class: 'M13.2069 10.4999C13.0194 10.3125 12.7651 10.2072 12.4999 10.2072C12.2348 10.2072 11.9805 10.3125 11.7929 10.4999L11.2929 10.9999H8.99994V6.99994H10.3629C10.2479 7.1876 10.1989 7.40832 10.2238 7.62701C10.2486 7.84571 10.3458 8.04983 10.4999 8.20694L11.2929 8.99994C11.4805 9.18741 11.7348 9.29273 11.9999 9.29273C12.2651 9.29273 12.5194 9.18741 12.7069 8.99994L13.9999 7.70694C14.1874 7.51941 14.2927 7.2651 14.2927 6.99994C14.2927 6.73478 14.1874 6.48047 13.9999 6.29294L13.2069 5.49994C13.0194 5.31247 12.7651 5.20715 12.4999 5.20715C12.2348 5.20715 11.9805 5.31247 11.7929 5.49994L11.2929 5.99994H6.70694L7.49994 5.20694C7.68741 5.01941 7.79273 4.7651 7.79273 4.49994C7.79273 4.23478 7.68741 3.98047 7.49994 3.79294L6.20694 2.49994C6.01941 2.31247 5.7651 2.20715 5.49994 2.20715C5.23478 2.20715 4.98047 2.31247 4.79294 2.49994L1.49994 5.79294C1.31247 5.98047 1.20715 6.23478 1.20715 6.49994C1.20715 6.7651 1.31247 7.01941 1.49994 7.20694L2.79294 8.49994C2.98047 8.68741 3.23478 8.79273 3.49994 8.79273C3.7651 8.79273 4.01941 8.68741 4.20694 8.49994L5.70694 6.99994H7.99994V11.4999C7.99994 11.6325 8.05262 11.7597 8.14639 11.8535C8.24015 11.9473 8.36733 11.9999 8.49994 11.9999H10.3629C10.2479 12.1876 10.1989 12.4083 10.2238 12.627C10.2486 12.8457 10.3458 13.0498 10.4999 13.2069L11.2929 13.9999C11.4805 14.1874 11.7348 14.2927 11.9999 14.2927C12.2651 14.2927 12.5194 14.1874 12.7069 13.9999L13.9999 12.7069C14.1874 12.5194 14.2927 12.2651 14.2927 11.9999C14.2927 11.7348 14.1874 11.4805 13.9999 11.2929L13.2069 10.4999ZM3.49994 7.79294L2.20694 6.49994L5.49994 3.20694L6.79294 4.49994L3.49994 7.79294ZM13.2929 6.99994L11.9999 8.29294L11.2069 7.49994L12.4999 6.20694L13.2929 6.99994ZM11.9999 13.2929L11.2069 12.4999L12.4999 11.2069L13.2929 11.9999L11.9999 13.2929Z',

  // symbol-method.svg (same as function - cube)
  Method: 'M4.69684 5.04043C4.44303 4.93166 4.14909 5.04923 4.04031 5.30305C3.93153 5.55686 4.04911 5.8508 4.30292 5.95958L7.49988 7.3297V10.5C7.49988 10.7761 7.72374 11 7.99988 11C8.27603 11 8.49988 10.7761 8.49988 10.5V7.3297L11.6968 5.95958C11.9507 5.8508 12.0682 5.55686 11.9595 5.30305C11.8507 5.04923 11.5567 4.93166 11.3029 5.04043L7.99988 6.45602L4.69684 5.04043ZM9.07694 1.37855C8.38373 1.11193 7.61627 1.11193 6.92306 1.37855L1.96153 3.28683C1.38224 3.50964 1 4.06619 1 4.68685V11.3133C1 11.9339 1.38224 12.4905 1.96153 12.7133L6.92306 14.6216C7.61627 14.8882 8.38373 14.8882 9.07694 14.6216L14.0385 12.7133C14.6178 12.4905 15 11.9339 15 11.3133V4.68685C15 4.06619 14.6178 3.50964 14.0385 3.28683L9.07694 1.37855ZM7.28204 2.3119C7.74418 2.13415 8.25582 2.13415 8.71796 2.3119L13.6795 4.22018C13.8726 4.29445 14 4.47997 14 4.68685V11.3133C14 11.5201 13.8726 11.7057 13.6795 11.7799L8.71796 13.6882C8.25582 13.866 7.74418 13.866 7.28204 13.6882L2.32051 11.7799C2.12741 11.7057 2 11.5201 2 11.3133V4.68685C2 4.47997 2.12741 4.29445 2.32051 4.22018L7.28204 2.3119Z',

  // symbol-structure.svg (grid blocks)
  Struct: 'M1 3C1 2.44772 1.44772 2 2 2H14C14.5523 2 15 2.44772 15 3V6C15 6.55228 14.5523 7 14 7H2C1.44772 7 1 6.55228 1 6V3ZM2 3H14V6H2L2 3ZM2 9C1.44772 9 1 9.44772 1 10V13C1 13.5523 1.44772 14 2 14H5C5.55228 14 6 13.5523 6 13V10C6 9.44772 5.55228 9 5 9H2ZM5 10H2V13H5V10ZM11 9C10.4477 9 10 9.44772 10 10V13C10 13.5523 10.4477 14 11 14H14C14.5523 14 15 13.5523 15 13V10C15 9.44772 14.5523 9 14 9H11ZM14 10H11V13H14V10Z',

  // symbol-interface.svg
  Interface: 'M2 3.5C2 2.67157 2.67157 2 3.5 2C4.32843 2 5 2.67157 5 3.5C5 4.06 4.69 4.55 4.23 4.82L4.23 6H5.5C6.88 6 8 7.12 8 8.5V10H9.5C9.22386 10 9 10.2239 9 10.5C9 10.7761 9.22386 11 9.5 11H12.5C12.7761 11 13 10.7761 13 10.5C13 10.2239 12.7761 10 12.5 10H11V8.5C11 7.12 12.12 6 13.5 6H14V4.82C13.54 4.55 13.23 4.06 13.23 3.5C13.23 2.67157 13.9016 2 14.73 2C15.5584 2 16.23 2.67157 16.23 3.5C16.23 4.06 15.92 4.55 15.46 4.82V6H13.5C12.67 6 12 6.67 12 7.5V10H9V7.5C9 6.67 8.33 6 7.5 6H4.23V4.82C4.69 4.55 5 4.06 5 3.5Z',

  // symbol-enum.svg
  Enum: 'M7 3L1 6V13L7 16L13 13V6L7 3ZM12 12.5L7 15L2 12.5V6.5L7 4L12 6.5V12.5ZM4 11.42L7 12.92L10 11.42V7.58L7 6.08L4 7.58V11.42ZM5 8.08L7 7.08L9 8.08V10.92L7 11.92L5 10.92V8.08Z',

  // symbol-constant.svg
  Constant: 'M4 6h8v1H4V6zm8 3H4v1h8V9zm-8 3h6v1H4v-1zm0-9h8v1H4V3zm-1-2a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V2a1 1 0 0 0-1-1H3zm0 1h10v12H3V2z',

  // symbol-variable.svg
  Variable: 'M5.60124 1.68422C5.49543 1.29668 5.09555 1.06676 4.70801 1.17258C4.32047 1.27839 4.09055 1.67827 4.19637 2.06581L5.19637 5.78444C5.42573 6.63835 6.19181 7.24999 7.07483 7.24999H8.92517C9.80819 7.24999 10.5743 6.63835 10.8036 5.78444L11.8036 2.06581C11.9095 1.67827 11.6795 1.27839 11.292 1.17258C10.9045 1.06676 10.5046 1.29668 10.3988 1.68422L9.39876 5.40285C9.32199 5.68936 9.04261 5.89999 8.92517 5.89999L7.07483 5.89999C6.95739 5.89999 6.67801 5.68936 6.60124 5.40285L5.60124 1.68422ZM4.79285 8.42858C5.09859 8.23624 5.2003 7.83835 5.00795 7.53261C4.81561 7.22687 4.41772 7.12517 4.11198 7.31751L1.11198 9.20847C0.729714 9.44905 0.5 9.87342 0.5 10.3309V13.0001C0.5 13.4576 0.729714 13.882 1.11198 14.1226L4.11198 16.0135C4.41772 16.2059 4.81561 16.1042 5.00795 15.7984C5.2003 15.4927 5.09859 15.0948 4.79285 14.9025L1.79285 13.0115C1.71465 12.9623 1.66667 12.8776 1.66667 12.7863V10.5447C1.66667 10.4535 1.71465 10.3688 1.79285 10.3196L4.79285 8.42858Z',

  // symbol-field.svg (dot with line)
  Field: 'M14.45 4.5H9.42997V3.5H14.45C14.86 3.5 15.2 3.83 15.2 4.25V11.75C15.2 12.17 14.86 12.5 14.45 12.5H9.42997V11.5H14.2V4.5H14.45ZM5.17997 4.5H1.79997V11.5H5.17997V12.5H1.54997C1.13997 12.5 0.799969 12.17 0.799969 11.75V4.25C0.799969 3.83 1.13997 3.5 1.54997 3.5H5.17997V4.5ZM10.3 8.5H5.60997V7.5H10.3V8.5Z',

  // symbol-property.svg
  Property: 'M2.807 2.111a1.027 1.027 0 0 0-.529.96v9.859c0 .393.218.752.567.935l5.157 2.678a1.027 1.027 0 0 0 .994-.007l5.156-2.678c.348-.183.566-.542.566-.935V3.071c0-.402-.237-.767-.606-.934L8.955 0.111A1.028 1.028 0 0 0 7.984.1L2.807 2.111zM8.5 1.137l4.706 1.917V12.43L8.5 14.78V8.039l-5.5-2.59V3.054L8.5 1.137z',

  // symbol-namespace.svg (module/box icon)
  Module: 'M4 2H12C12.5523 2 13 2.44772 13 3V13C13 13.5523 12.5523 14 12 14H4C3.44772 14 3 13.5523 3 13V3C3 2.44772 3.44772 2 4 2ZM4 3V13H12V3H4ZM5 5H11V6H5V5ZM5 8H11V9H5V8ZM5 11H8V12H5V11Z',

  // symbol-file.svg
  File: 'M10.57 1.14L13.85 4.43C13.9447 4.52449 13.998 4.65263 13.998 4.78601C13.998 4.91939 13.9447 5.04753 13.85 5.14202L13.14 5.85202C13.0455 5.94672 12.9174 5.99996 12.784 5.99996C12.6506 5.99996 12.5225 5.94672 12.428 5.85202L10.14 3.57L9.14 4.57V13.5C9.14 13.7761 8.91614 14 8.64 14H2.5C2.22386 14 2 13.7761 2 13.5V2.5C2 2.22386 2.22386 2 2.5 2H9.39L10.57 1.14Z',

  // Import (arrow-down + line)
  Import: 'M8 1v8.59l3.3-3.3.7.71-4.5 4.5L3 7l.71-.71L7 9.59V1h1zm-5 12v1h10v-1H3z',

  // Constructor (wrench-like)
  Constructor: 'M4 12V6l4-3 4 3v6H9V9H7v3H4zM8 4.5L5 6.75V11h1V8h4v3h1V6.75L8 4.5z',

  // Trait (circle with T)
  Trait: 'M8 1a7 7 0 110 14A7 7 0 018 1zm0 1a6 6 0 100 12A6 6 0 008 2zM5.5 5h5v1H8.75v5h-1.5V6H5.5V5z',

  // Decorator (@)
  Decorator: 'M8 1a7 7 0 110 14A7 7 0 018 1zm0 1a6 6 0 100 12A6 6 0 008 2zm0 2.5a3.5 3.5 0 013.5 3.5c0 1.1-.5 1.9-1.3 2.2-.3.1-.6 0-.7-.2V8c0-.8-.7-1.5-1.5-1.5S6.5 7.2 6.5 8s.7 1.5 1.5 1.5c.4 0 .7-.1 1-.4v.5c0 .5.3.9.7 1.1.2.1.4.1.6.1.3 0 .7-.1 1-.3A3.5 3.5 0 008 4.5z',

  // EnumVariant
  EnumVariant: 'M3 4h1.5a1.5 1.5 0 110 3H3V4zm0 5h1.5a1.5 1.5 0 110 3H3V9zm6-5h4.5v1H9V4zm0 5h4.5v1H9V9zm0 3h3v1H9v-1z',

  // TypeAlias (T=)
  TypeAlias: 'M5 3h6v1.5H9V12H7V4.5H5V3zm6.5 7h3v1h-3v-1zm0 2h3v1h-3v-1z',
};

export function kindIcon(kind: string, size = 16): string {
  const path = CODICON_PATHS[kind];
  if (!path) {
    return `<svg viewBox="0 0 16 16" width="${size}" height="${size}" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1"/><text x="8" y="11" text-anchor="middle" font-size="7" font-weight="700" font-family="sans-serif">${kind.charAt(0)}</text></svg>`;
  }
  return `<svg viewBox="0 0 16 16" width="${size}" height="${size}" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="${path}"/></svg>`;
}


// ── File Extension Icons ──

const EXT_TO_LANG: Record<string, string> = {
  py: 'Python',
  ts: 'TypeScript', tsx: 'TypeScript', mts: 'TypeScript', cts: 'TypeScript',
  js: 'JavaScript', jsx: 'JavaScript', mjs: 'JavaScript', cjs: 'JavaScript',
  rs: 'Rust',
  go: 'Go',
  java: 'Java',
  cpp: 'Cpp', cc: 'Cpp', cxx: 'Cpp', hpp: 'Cpp',
  c: 'C', h: 'C',
  cs: 'CSharp',
  rb: 'Ruby',
  php: 'PHP',
  kt: 'Kotlin', kts: 'Kotlin',
  svelte: 'Svelte',
  md: 'Markdown', mdx: 'Markdown',
  html: 'HTML', htm: 'HTML',
  css: 'CSS',
  scss: 'CSS', sass: 'CSS',
};

// Config/data file icons (inline SVG)
const SPECIAL_ICONS: Record<string, string> = {
  json: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="14" height="14" rx="2" fill="#292929"/><text x="8" y="10.5" text-anchor="middle" fill="#F5C842" font-size="5" font-weight="700" font-family="monospace">{}</text></svg>`,
  toml: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="14" height="14" rx="2" fill="#292929"/><text x="8" y="10.8" text-anchor="middle" fill="#9B9B9B" font-size="4.5" font-weight="700" font-family="monospace">TOML</text></svg>`,
  yaml: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="14" height="14" rx="2" fill="#292929"/><text x="8" y="10.8" text-anchor="middle" fill="#CB171E" font-size="4.5" font-weight="700" font-family="monospace">YML</text></svg>`,
  yml: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="14" height="14" rx="2" fill="#292929"/><text x="8" y="10.8" text-anchor="middle" fill="#CB171E" font-size="4.5" font-weight="700" font-family="monospace">YML</text></svg>`,
  lock: `<svg viewBox="0 0 16 16" fill="none" stroke="#555" stroke-width="1.3" xmlns="http://www.w3.org/2000/svg"><rect x="3" y="7" width="10" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 015 0v2"/><circle cx="8" cy="10.5" r="1" fill="#555"/></svg>`,
  svg: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="14" height="14" rx="2" fill="#292929"/><text x="8" y="10.8" text-anchor="middle" fill="#FFB13B" font-size="4.5" font-weight="700" font-family="monospace">SVG</text></svg>`,
  png: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="2" y="2" width="12" height="12" rx="1.5" stroke="#6A9955" stroke-width="1.2"/><circle cx="6" cy="6" r="1.5" fill="#6A9955"/><path d="M3 11l3-3 2 2 2-3 3 4" stroke="#6A9955" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>`,
  jpg: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="2" y="2" width="12" height="12" rx="1.5" stroke="#6A9955" stroke-width="1.2"/><circle cx="6" cy="6" r="1.5" fill="#6A9955"/><path d="M3 11l3-3 2 2 2-3 3 4" stroke="#6A9955" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>`,
  jpeg: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><rect x="2" y="2" width="12" height="12" rx="1.5" stroke="#6A9955" stroke-width="1.2"/><circle cx="6" cy="6" r="1.5" fill="#6A9955"/><path d="M3 11l3-3 2 2 2-3 3 4" stroke="#6A9955" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>`,
  gitignore: `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><circle cx="8" cy="8" r="6" fill="#F05032" opacity="0.9"/><path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="#fff" stroke-width="1.3" stroke-linecap="round"/></svg>`,
  env: `<svg viewBox="0 0 16 16" fill="none" stroke="#F5C842" stroke-width="1.3" xmlns="http://www.w3.org/2000/svg"><rect x="2" y="3" width="12" height="10" rx="1.5"/><circle cx="8" cy="8" r="2"/><path d="M3 8h3M10 8h3"/></svg>`,
};

const GENERIC_FILE = `<svg viewBox="0 0 16 16" fill="none" stroke="#555" stroke-width="1.2" xmlns="http://www.w3.org/2000/svg"><path d="M4 2h5l4 4v8H4V2z"/><path d="M9 2v4h4"/></svg>`;

export function fileIcon(filename: string, size = 16): string {
  const ext = filename.includes('.') ? filename.split('.').pop()!.toLowerCase() : '';

  // Special file names
  if (filename === '.gitignore' || filename === '.gitkeep') {
    return SPECIAL_ICONS.gitignore.replace('<svg ', `<svg width="${size}" height="${size}" `);
  }
  if (filename === '.env' || filename.startsWith('.env.')) {
    return SPECIAL_ICONS.env.replace('<svg ', `<svg width="${size}" height="${size}" `);
  }
  if (SPECIAL_ICONS[ext]) {
    return SPECIAL_ICONS[ext].replace('<svg ', `<svg width="${size}" height="${size}" `);
  }

  const lang = EXT_TO_LANG[ext];
  if (lang) return langIcon(lang, size);

  return GENERIC_FILE.replace('<svg ', `<svg width="${size}" height="${size}" `);
}


// ── Folder Icons ──

const FOLDER_OPEN = `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M1.5 3.5A1 1 0 012.5 2.5h3l1.5 1.5h5.5a1 1 0 011 1v1H2.5l-1-1v-1z" fill="#F6821F" opacity="0.5"/><path d="M1.5 5.5h12l-1.5 8h-9z" fill="#F6821F" opacity="0.8"/></svg>`;
const FOLDER_CLOSED = `<svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#F6821F" opacity="0.6"/></svg>`;

const SPECIAL_FOLDERS: Record<string, string> = {
  src:          `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#42A5F5" opacity="0.6"/></svg>`,
  lib:          `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#AB47BC" opacity="0.6"/></svg>`,
  test:         `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#66BB6A" opacity="0.6"/></svg>`,
  tests:        `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#66BB6A" opacity="0.6"/></svg>`,
  node_modules: `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#8BC34A" opacity="0.4"/></svg>`,
  dist:         `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#FFA726" opacity="0.5"/></svg>`,
  build:        `<svg viewBox="0 0 16 16" fill="none"><path d="M2 3a1 1 0 011-1h3.4l1.5 1.5H13a1 1 0 011 1V12a1 1 0 01-1 1H3a1 1 0 01-1-1V3z" fill="#FFA726" opacity="0.5"/></svg>`,
};

export function folderIcon(name: string, open: boolean, size = 16): string {
  const special = SPECIAL_FOLDERS[name];
  if (special) return special.replace('<svg ', `<svg width="${size}" height="${size}" `);
  const svg = open ? FOLDER_OPEN : FOLDER_CLOSED;
  return svg.replace('<svg ', `<svg width="${size}" height="${size}" `);
}

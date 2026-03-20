export interface MacOSPalette {
  tone: string;
  fieldClassName: string;
  glowClassName: string;
  heroClassName: string;
  badgeClassName: string;
  buttonHoverClassName: string;
}

const DEFAULT_PALETTE: MacOSPalette = {
  tone: 'Graphite',
  fieldClassName: 'from-slate-400/28 via-slate-300/10 to-transparent',
  glowClassName: 'bg-slate-300/18',
  heroClassName: 'border-slate-300/16 bg-[linear-gradient(135deg,rgba(148,163,184,0.18),rgba(15,23,42,0.84)_42%,rgba(15,23,42,0.96)_100%)]',
  badgeClassName: 'border-slate-300/22 bg-slate-300/10 text-slate-100/85',
  buttonHoverClassName: 'hover:bg-slate-50',
};

export function getMacOSPalette(versionName: string): MacOSPalette {
  const normalized = versionName.toLowerCase();

  if (normalized.includes('tahoe')) {
    return {
      tone: 'Alpine Blue',
      fieldClassName: 'from-sky-300/35 via-blue-400/18 to-transparent',
      glowClassName: 'bg-sky-300/20',
      heroClassName: 'border-sky-300/18 bg-[linear-gradient(135deg,rgba(125,211,252,0.22),rgba(30,64,175,0.22),rgba(15,23,42,0.9)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-sky-200/24 bg-sky-300/10 text-sky-100/88',
      buttonHoverClassName: 'hover:bg-sky-50',
    };
  }

  if (normalized.includes('sequoia')) {
    return {
      tone: 'Forest Light',
      fieldClassName: 'from-emerald-300/30 via-cyan-300/16 to-transparent',
      glowClassName: 'bg-emerald-300/16',
      heroClassName: 'border-emerald-300/16 bg-[linear-gradient(135deg,rgba(110,231,183,0.22),rgba(34,211,238,0.16),rgba(15,23,42,0.88)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-emerald-200/24 bg-emerald-300/10 text-emerald-100/88',
      buttonHoverClassName: 'hover:bg-emerald-50',
    };
  }

  if (normalized.includes('sonoma')) {
    return {
      tone: 'Rose Dune',
      fieldClassName: 'from-fuchsia-300/34 via-rose-300/18 to-transparent',
      glowClassName: 'bg-fuchsia-300/18',
      heroClassName: 'border-fuchsia-300/16 bg-[linear-gradient(135deg,rgba(244,114,182,0.24),rgba(251,191,36,0.14),rgba(15,23,42,0.88)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-fuchsia-200/24 bg-fuchsia-300/10 text-fuchsia-100/88',
      buttonHoverClassName: 'hover:bg-fuchsia-50',
    };
  }

  if (normalized.includes('ventura')) {
    return {
      tone: 'Indigo Bloom',
      fieldClassName: 'from-indigo-300/34 via-violet-300/18 to-transparent',
      glowClassName: 'bg-indigo-300/18',
      heroClassName: 'border-indigo-300/16 bg-[linear-gradient(135deg,rgba(129,140,248,0.24),rgba(236,72,153,0.14),rgba(15,23,42,0.89)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-indigo-200/24 bg-indigo-300/10 text-indigo-100/88',
      buttonHoverClassName: 'hover:bg-indigo-50',
    };
  }

  if (normalized.includes('monterey')) {
    return {
      tone: 'Coral Mist',
      fieldClassName: 'from-rose-300/34 via-orange-300/18 to-transparent',
      glowClassName: 'bg-rose-300/18',
      heroClassName: 'border-rose-300/16 bg-[linear-gradient(135deg,rgba(251,113,133,0.22),rgba(251,191,36,0.14),rgba(15,23,42,0.89)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-rose-200/24 bg-rose-300/10 text-rose-100/88',
      buttonHoverClassName: 'hover:bg-rose-50',
    };
  }

  if (normalized.includes('big sur')) {
    return {
      tone: 'Pacific Blue',
      fieldClassName: 'from-cyan-300/34 via-sky-300/18 to-transparent',
      glowClassName: 'bg-cyan-300/18',
      heroClassName: 'border-cyan-300/16 bg-[linear-gradient(135deg,rgba(103,232,249,0.22),rgba(59,130,246,0.18),rgba(15,23,42,0.88)_44%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-cyan-200/24 bg-cyan-300/10 text-cyan-100/88',
      buttonHoverClassName: 'hover:bg-cyan-50',
    };
  }

  if (normalized.includes('catalina')) {
    return {
      tone: 'Catalina Night',
      fieldClassName: 'from-blue-300/32 via-teal-300/14 to-transparent',
      glowClassName: 'bg-blue-300/18',
      heroClassName: 'border-blue-300/16 bg-[linear-gradient(135deg,rgba(96,165,250,0.2),rgba(45,212,191,0.14),rgba(15,23,42,0.9)_46%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-blue-200/24 bg-blue-300/10 text-blue-100/88',
      buttonHoverClassName: 'hover:bg-blue-50',
    };
  }

  if (normalized.includes('mojave')) {
    return {
      tone: 'Desert Glow',
      fieldClassName: 'from-amber-300/32 via-orange-300/16 to-transparent',
      glowClassName: 'bg-amber-300/18',
      heroClassName: 'border-amber-300/16 bg-[linear-gradient(135deg,rgba(252,211,77,0.22),rgba(249,115,22,0.16),rgba(15,23,42,0.9)_46%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-amber-200/24 bg-amber-300/10 text-amber-100/88',
      buttonHoverClassName: 'hover:bg-amber-50',
    };
  }

  if (normalized.includes('high sierra')) {
    return {
      tone: 'Sierra Sunset',
      fieldClassName: 'from-orange-300/30 via-pink-300/16 to-transparent',
      glowClassName: 'bg-orange-300/16',
      heroClassName: 'border-orange-300/16 bg-[linear-gradient(135deg,rgba(253,186,116,0.22),rgba(244,114,182,0.14),rgba(15,23,42,0.9)_46%,rgba(15,23,42,0.97)_100%)]',
      badgeClassName: 'border-orange-200/24 bg-orange-300/10 text-orange-100/88',
      buttonHoverClassName: 'hover:bg-orange-50',
    };
  }

  return DEFAULT_PALETTE;
}

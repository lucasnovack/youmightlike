// Application visual configuration, domain colors, and badge helpers (Apple HIG)

export const DOMAIN_COLORS = {
  seed: '#1d1d1f',         // Deep Apple Slate/Black for Seeds
  anti_seed: '#ef4444',    // Apple Crimson for Anti-Seeds (repulsion)
  antiSeed: '#ef4444',
  artist: '#c2410c',       // Apple Rust/Orange for Artists
  track: '#0284c7',        // Apple Sky for Music Tracks
  game: '#7c3aed',         // Apple Violet for Games
  movie: '#b45309',        // Apple Amber for Movies
  series: '#9333ea',       // Apple Purple for Series
  developer: '#475569',    // Apple Slate for Game Studios
  keyword: '#64748b',      // Apple Slate for Bridges/Tags
  bridgeShared: '#334155', // Deeper Slate for multi-seed shared bridges
};

export function getRawType(node) {
  if (!node) return 'item';
  if (node.id && node.id.includes(':')) return node.id.split(':')[0];
  return node.type || 'item';
}

export function getTypeIcon(_type) {
  return '';
}

export function getSeedDotColor(type) {
  return DOMAIN_COLORS[type] || DOMAIN_COLORS.seed;
}

export function getTypeBadgeClass(type) {
  switch (type) {
    case 'seed':
      return 'text-slate-900 bg-slate-100 border-slate-300 font-semibold';
    case 'anti_seed':
    case 'antiSeed':
      return 'text-rose-700 bg-rose-50 border-rose-200 font-semibold';
    case 'game':
      return 'text-purple-700 bg-purple-50 border-purple-200/80 font-semibold';
    case 'artist':
      return 'text-orange-700 bg-orange-50 border-orange-200/80 font-semibold';
    case 'track':
      return 'text-sky-700 bg-sky-50 border-sky-200/80 font-semibold';
    case 'movie':
      return 'text-amber-800 bg-amber-50 border-amber-200/80 font-semibold';
    case 'series':
      return 'text-purple-700 bg-purple-50 border-purple-200/80 font-semibold';
    case 'keyword':
      return 'text-slate-600 bg-slate-100 border-slate-200 font-medium';
    default:
      return 'text-slate-600 bg-slate-50 border-slate-200 font-medium';
  }
}

export function getAffinityLabel(score, t) {
  const val = Number(score) || 0;
  if (val >= 5.0) return t('affinityVeryHigh');
  if (val >= 2.0) return t('affinityHigh');
  if (val >= 0.6) return t('affinityModerate');
  return t('affinityThematic');
}

export function getAffinityBadgeClass(score) {
  const val = Number(score) || 0;
  if (val >= 5.0) return 'text-blue-700 bg-blue-50 border-blue-200/80 font-medium';
  if (val >= 2.0) return 'text-teal-700 bg-teal-50 border-teal-200/80 font-medium';
  if (val >= 0.6) return 'text-slate-700 bg-slate-100 border-slate-200 font-medium';
  return 'text-slate-500 bg-slate-50 border-slate-200 font-normal';
}

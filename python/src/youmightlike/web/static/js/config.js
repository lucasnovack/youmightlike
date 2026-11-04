// Application visual configuration, domain colors, and badge helpers

export const DOMAIN_COLORS = {
  seed: '#6366f1',     // Distinct Indigo/Violet for Seeds (user inputs)
  artist: '#ea580c',   // Warm Amber/Orange for Recommended Artists
  track: '#0d9488',    // Emerald Teal for Music Tracks
  game: '#2563eb',     // Royal Blue for Games
  movie: '#d97706',    // Golden Amber for Movies
  series: '#9333ea',   // Rich Purple for Series
  developer: '#475569',// Slate for Game Studios
  keyword: '#64748b',  // Subtle Slate for Bridges/Tags
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
      return 'text-indigo-700 bg-indigo-50 border-indigo-200';
    case 'game':
      return 'text-blue-700 bg-blue-50 border-blue-200';
    case 'artist':
      return 'text-orange-700 bg-orange-50 border-orange-200';
    case 'track':
      return 'text-teal-700 bg-teal-50 border-teal-200';
    case 'movie':
      return 'text-amber-700 bg-amber-50 border-amber-200';
    case 'series':
      return 'text-purple-700 bg-purple-50 border-purple-200';
    case 'keyword':
      return 'text-slate-600 bg-slate-100 border-slate-200';
    default:
      return 'text-slate-600 bg-slate-50 border-slate-200';
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
  if (val >= 5.0) return 'text-indigo-700 bg-indigo-50 border-indigo-200';
  if (val >= 2.0) return 'text-teal-700 bg-teal-50 border-teal-200';
  if (val >= 0.6) return 'text-blue-700 bg-blue-50 border-blue-200';
  return 'text-slate-600 bg-slate-100 border-slate-200';
}

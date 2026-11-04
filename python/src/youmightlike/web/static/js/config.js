// Application visual configuration, domain colors, and badge helpers

export const DOMAIN_COLORS = {
  game: '#2563eb',
  artist: '#ea580c',
  track: '#0d9488',
  movie: '#ca8a04',
  series: '#7c3aed',
  developer: '#334155',
  keyword: '#1e293b',
  seed: '#e11d48',
};

export function getRawType(node) {
  if (!node) return 'item';
  if (node.id && node.id.includes(':')) return node.id.split(':')[0];
  return node.type || 'item';
}

export function getTypeBadgeClass(type) {
  switch (type) {
    case 'game':
      return 'text-canvas-denim bg-canvas-denim/10 border-canvas-denim/25';
    case 'artist':
      return 'text-canvas-orange bg-canvas-orange/10 border-canvas-orange/30';
    case 'track':
      return 'text-canvas-sage bg-canvas-sage/10 border-canvas-sage/25';
    case 'movie':
      return 'text-canvas-amber bg-canvas-amber/10 border-canvas-amber/25';
    case 'series':
      return 'text-canvas-violet bg-canvas-violet/10 border-canvas-violet/25';
    case 'keyword':
      return 'text-slate-700 bg-slate-200/70 border-slate-300';
    default:
      return 'text-canvas-muted bg-canvas-sidebar border-canvas-border';
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
  if (val >= 5.0) return 'text-canvas-sage bg-canvas-sage/15 border-canvas-sage/30';
  if (val >= 2.0) return 'text-canvas-denim bg-canvas-denim/10 border-canvas-denim/30';
  if (val >= 0.6) return 'text-canvas-text bg-canvas-sidebar border-canvas-borderDark';
  return 'text-canvas-muted bg-canvas-sidebar border-canvas-border';
}

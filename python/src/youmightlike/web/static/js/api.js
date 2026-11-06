export async function searchEntities(query, entityType = '', signal = null) {
  let url = `/api/search?q=${encodeURIComponent(query)}`;
  if (entityType) {
    url += `&entity_type=${encodeURIComponent(entityType)}`;
  }
  const res = await fetch(url, { signal });
  if (!res.ok) {
    throw new Error(`Search request failed with status: ${res.status}`);
  }
  const data = await res.json();
  if (res.headers.get('X-YouMightLike-Offline') === '1') {
    data.offlineCached = true;
  }
  return data;
}

export async function fetchRecommendations({ seeds, targetType = null, limit = 24, lang = 'pt', signal = null }) {
  const payload = {
    seeds: [...seeds],
    target_type: targetType || null,
    limit,
    lang,
  };

  const res = await fetch('/api/recommend', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    signal,
  });

  if (!res.ok) {
    throw new Error(`Recommendation request failed with status: ${res.status}`);
  }

  const data = await res.json();
  if (res.headers.get('X-YouMightLike-Offline') === '1') {
    data.offlineCached = true;
  }
  return data;
}


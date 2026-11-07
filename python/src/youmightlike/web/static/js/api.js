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

export async function fetchRecommendations({
  seeds,
  negativeSeeds = [],
  targetType = null,
  limit = 24,
  lang = 'pt',
  beta = 1.2,
  signal = null,
}) {
  const payload = {
    seeds: [...seeds],
    negative_seeds: [...(negativeSeeds || [])],
    target_type: targetType || null,
    limit,
    lang,
    beta,
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


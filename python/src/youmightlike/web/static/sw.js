/**
 * youmightlike — Offline Service Worker
 * Architecture: App Shell (Cache-First / Stale-While-Revalidate) + Dynamic API Caching
 */

const CACHE_VERSION = 'v1.1.0';
const STATIC_CACHE = `ym-static-${CACHE_VERSION}`;
const API_CACHE = `ym-api-${CACHE_VERSION}`;

// Core static assets required for the App Shell (100% Local-First)
const PRECACHE_LOCAL = [
  '/',
  '/index.html',
  '/manifest.json',
  '/vendor/tailwindcss.js',
  '/vendor/vue.global.prod.js',
  '/vendor/d3.min.js',
  '/static/css/base.css',
  '/static/css/components.css',
  '/static/css/constellation.css',
  '/static/js/tailwind-config.js',
  '/static/js/i18n.js',
  '/static/js/config.js',
  '/static/js/api.js',
  '/static/js/constellation.js',
  '/static/js/receipt.js',
  '/static/js/app.js',
  '/static/icons/icon.svg',
  '/static/icons/icon-192.png',
  '/static/icons/icon-512.png',
];

// External web fonts (optional progressive enhancement)
const PRECACHE_CDN = [
  'https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,300;0,400;0,500;0,600;0,700;1,400&display=swap',
];

// Install: Cache critical static shell
self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(STATIC_CACHE).then(async (cache) => {
      // 1. Precache local application files
      await Promise.allSettled(
        PRECACHE_LOCAL.map(async (url) => {
          try {
            const req = new Request(url, { cache: 'reload' });
            const res = await fetch(req);
            if (res.ok) {
              await cache.put(req, res);
            }
          } catch (err) {
            console.warn('[SW] Falha ao pre-cachear asset local:', url, err);
          }
        })
      );

      // 2. Precache vendor CDN dependencies
      await Promise.allSettled(
        PRECACHE_CDN.map(async (url) => {
          try {
            const res = await fetch(url, { mode: 'cors' });
            if (res.ok) {
              await cache.put(url, res);
            }
          } catch (err) {
            console.warn('[SW] Falha ao pre-cachear dependência CDN:', url, err);
          }
        })
      );
    }).then(() => self.skipWaiting())
  );
});

// Activate: Prune stale caches and claim clients immediately
self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((keys) => {
      return Promise.all(
        keys
          .filter((key) => key !== STATIC_CACHE && key !== API_CACHE)
          .map((key) => {
            console.log('[SW] Removendo cache obsoleto:', key);
            return caches.delete(key);
          })
      );
    }).then(() => self.clients.claim())
  );
});

// Fetch event router
self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);

  // 1. Navigation requests (App Shell)
  if (request.mode === 'navigate') {
    event.respondWith(handleNavigation(request));
    return;
  }

  // 2. Search API (GET /api/search)
  if (url.pathname === '/api/search') {
    event.respondWith(handleSearchApi(request));
    return;
  }

  // 3. Recommendation API (POST /api/recommend)
  if (url.pathname === '/api/recommend' && request.method === 'POST') {
    event.respondWith(handleRecommendApi(request));
    return;
  }

  // 4. Static assets & CDN (Stale-While-Revalidate)
  if (request.method === 'GET') {
    event.respondWith(handleStaleWhileRevalidate(request));
  }
});

/**
 * Handle navigation requests: Network first with cached index.html fallback
 */
async function handleNavigation(request) {
  try {
    const networkResponse = await fetch(request);
    if (networkResponse && networkResponse.ok) {
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, networkResponse.clone());
      return networkResponse;
    }
  } catch (error) {
    // Network failed, fall back to cached shell
  }

  const cachedIndex = await caches.match('/index.html') || await caches.match('/');
  if (cachedIndex) {
    return cachedIndex;
  }

  return new Response('Offline: Aplicativo não disponível sem conexão.', {
    status: 503,
    statusText: 'Service Unavailable',
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}

/**
 * Handle GET /api/search with network-first and cache fallback
 */
async function handleSearchApi(request) {
  const apiCache = await caches.open(API_CACHE);

  try {
    const response = await fetch(request);
    if (response && response.ok) {
      apiCache.put(request, response.clone());
    }
    return response;
  } catch (networkError) {
    const cached = await apiCache.match(request);
    if (cached) {
      // Retorna cópia com header informando cache offline
      const cachedHeaders = new Headers(cached.headers);
      cachedHeaders.set('X-YouMightLike-Offline', '1');
      return new Response(await cached.blob(), {
        status: cached.status,
        statusText: cached.statusText,
        headers: cachedHeaders,
      });
    }

    const url = new URL(request.url);
    const query = url.searchParams.get('q') || '';
    const fallbackPayload = {
      query,
      results: [],
      offline: true,
      message: 'Sem conexão à rede. Busca não encontrada no cache.',
    };
    return new Response(JSON.stringify(fallbackPayload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }
}

/**
 * Handle POST /api/recommend
 * Creates a deterministic synthetic cache key based on the JSON payload
 */
async function handleRecommendApi(request) {
  const apiCache = await caches.open(API_CACHE);

  let bodyText = '';
  try {
    const cloned = request.clone();
    bodyText = await cloned.text();
  } catch (e) {
    bodyText = '';
  }

  // Gera chave canônica a partir do payload
  const syntheticUrl = '/api/recommend?query=' + encodeURIComponent(bodyText);
  const syntheticReq = new Request(syntheticUrl);

  try {
    const response = await fetch(request);
    if (response && response.ok) {
      // Salva no cache com a chave sintética
      apiCache.put(syntheticReq, response.clone());
    }
    return response;
  } catch (networkError) {
    const cached = await apiCache.match(syntheticReq);
    if (cached) {
      const cachedHeaders = new Headers(cached.headers);
      cachedHeaders.set('X-YouMightLike-Offline', '1');
      return new Response(await cached.blob(), {
        status: cached.status,
        statusText: cached.statusText,
        headers: cachedHeaders,
      });
    }

    const fallbackPayload = {
      seeds: [],
      results: [],
      graph: { nodes: [], links: [] },
      offline: true,
      message: 'Sem conexão à rede e recomendação não encontrada no cache local.',
    };
    return new Response(JSON.stringify(fallbackPayload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }
}

/**
 * Stale-While-Revalidate strategy for static resources & CDN
 */
async function handleStaleWhileRevalidate(request) {
  const staticCache = await caches.open(STATIC_CACHE);
  const cachedResponse = await staticCache.match(request);

  // Faz fetch em segundo plano para atualizar o cache
  const fetchPromise = fetch(request)
    .then((networkResponse) => {
      if (networkResponse && networkResponse.ok) {
        staticCache.put(request, networkResponse.clone());
      }
      return networkResponse;
    })
    .catch(() => null);

  // Se já temos em cache, retorna imediatamente
  if (cachedResponse) {
    return cachedResponse;
  }

  // Senão, aguarda a rede
  const networkResponse = await fetchPromise;
  if (networkResponse) {
    return networkResponse;
  }

  // Se a rede também falhou e não temos cache
  return new Response('', { status: 408, statusText: 'Request Timed Out (Offline)' });
}

// Client-to-Worker Message handler
self.addEventListener('message', (event) => {
  if (event.data && event.data.action === 'skipWaiting') {
    self.skipWaiting();
  }
});

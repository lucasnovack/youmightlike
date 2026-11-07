// Main Vue 3 Application Controller for youmightlike
import { createTranslator } from './i18n.js?v=2.8';
import { getTypeBadgeClass, getTypeIcon, getSeedDotColor, getAffinityLabel, getAffinityBadgeClass, getRawType } from './config.js?v=2.8';
import { searchEntities, fetchRecommendations } from './api.js?v=2.8';
import { ConstellationEngine } from './constellation.js?v=2.8';

const { createApp, ref, computed, onMounted } = Vue;

createApp({
  setup() {
    const currentLang = ref(localStorage.getItem('ym_lang') || 'pt');
    const t = createTranslator(currentLang);

    const isOffline = ref(!navigator.onLine);
    const isOfflineCached = ref(false);

    const sidebarMode = ref('discoveries');
    const searchQuery = ref('');
    const searchType = ref('');
    const searchResults = ref([]);
    const isSearching = ref(false);
    const hasSearched = ref(false);
    const showLegend = ref(false);

    const profileSeeds = ref([]);
    const negativeSeeds = ref([]);
    const negativeBeta = ref(1.2);
    const knownItems = ref([]);
    const recommendations = ref([]);
    const isLoadingRecs = ref(false);
    const targetFilter = ref(null);

    // Inspector selection and recommendations
    const selectedNodeIds = ref([]);
    const inspectorRecs = ref([]);
    const inspectorFilter = ref(null);
    const isLoadingInspectorRecs = ref(false);

    const onlySharedBridges = ref(false);
    const hideTrackNodes = ref(false);
    const layoutMode = ref('galaxy'); // 'galaxy' (Obsidian) | 'flow' (Sankey)
    const graphSpacing = ref('comfortable'); // 'compact', 'comfortable', 'spacious'

    const setLayoutMode = (mode) => {
      layoutMode.value = mode;
      rebuildConstellation();
    };

    const getSpacingScale = (spacing) => {
      if (spacing === 'compact') return 1.05;
      if (spacing === 'spacious') return 1.70;
      return 1.35; // comfortable
    };

    const setGraphSpacing = (spacing) => {
      graphSpacing.value = spacing;
      rebuildConstellation();
    };

    let searchAbortController = null;
    let fullServerGraph = { nodes: [], links: [] };
    let constellation = null;

    // Translation & Language
    const setLang = (lang) => {
      currentLang.value = lang;
      localStorage.setItem('ym_lang', lang);
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
      if (selectedNodeIds.value.length > 0) {
        fetchLiveInspectorRecommendations();
      }
    };

    // Profile Persistence
    const loadSavedProfile = () => {
      const saved = localStorage.getItem('ym_taste_profile');
      if (saved) {
        try {
          const parsed = JSON.parse(saved);
          if (Array.isArray(parsed)) {
            const seenIds = new Set();
            const seenWorkKeys = new Set();
            const deduped = [];
            for (const s of parsed) {
              if (!s || !s.id || seenIds.has(s.id)) continue;
              const workKey = `${s.type || ''}:${(s.name || '').trim().toLowerCase()}`;
              if (s.name && seenWorkKeys.has(workKey)) continue;
              seenIds.add(s.id);
              if (s.name) seenWorkKeys.add(workKey);
              deduped.push(s);
            }
            profileSeeds.value = deduped;
          }
        } catch (e) {
          console.error('Failed to parse saved taste profile:', e);
        }
      }

      const savedNeg = localStorage.getItem('ym_negative_seeds');
      if (savedNeg) {
        try {
          const parsedNeg = JSON.parse(savedNeg);
          if (Array.isArray(parsedNeg)) {
            const seenIds = new Set();
            const dedupedNeg = [];
            for (const s of parsedNeg) {
              if (!s || !s.id || seenIds.has(s.id)) continue;
              seenIds.add(s.id);
              dedupedNeg.push(s);
            }
            negativeSeeds.value = dedupedNeg;
          }
        } catch (e) {
          console.error('Failed to parse saved negative seeds:', e);
        }
      }

      const savedBeta = localStorage.getItem('ym_negative_beta');
      if (savedBeta) {
        negativeBeta.value = parseFloat(savedBeta) || 1.2;
      }

      const savedKnown = localStorage.getItem('ym_known_items');
      if (savedKnown) {
        try {
          const parsedKnown = JSON.parse(savedKnown);
          if (Array.isArray(parsedKnown)) {
            const seenIds = new Set();
            const dedupedKnown = [];
            for (const k of parsedKnown) {
              if (!k || !k.id || seenIds.has(k.id)) continue;
              seenIds.add(k.id);
              dedupedKnown.push(k);
            }
            knownItems.value = dedupedKnown;
          }
        } catch (e) {
          console.error('Failed to parse saved known items:', e);
        }
      }
    };

    const saveProfile = () => {
      localStorage.setItem('ym_taste_profile', JSON.stringify(profileSeeds.value));
      localStorage.setItem('ym_negative_seeds', JSON.stringify(negativeSeeds.value));
      localStorage.setItem('ym_negative_beta', String(negativeBeta.value));
      localStorage.setItem('ym_known_items', JSON.stringify(knownItems.value));
    };

    const getSearchPlaceholder = () => {
      if (searchType.value === 'game') return t('searchGame');
      if (searchType.value === 'artist') return t('searchArtist');
      if (searchType.value === 'track') return t('searchTrack');
      if (searchType.value === 'movie') return t('searchMovie');
      if (searchType.value === 'series') return t('searchSeries');
      return t('searchAll');
    };

    // Category Counts & Grouping
    const domainCounts = computed(() => {
      const counts = { game: 0, music: 0, movie: 0, series: 0 };
      for (const r of recommendations.value) {
        if (r.type === 'game') counts.game++;
        else if (r.type === 'artist' || r.type === 'track') counts.music++;
        else if (r.type === 'movie') counts.movie++;
        else if (r.type === 'series') counts.series++;
      }
      return counts;
    });

    const groupedRecommendations = computed(() => {
      if (targetFilter.value !== null) {
        return [{
          key: 'filtered',
          label: targetFilter.value.toUpperCase(),
          color: '#0f172a',
          filterType: null,
          items: recommendations.value,
        }];
      }
      const games = recommendations.value.filter(r => r.type === 'game');
      const artists = recommendations.value.filter(r => r.type === 'artist');
      const tracks = recommendations.value.filter(r => r.type === 'track');
      const screen = recommendations.value.filter(r => r.type === 'movie' || r.type === 'series' || r.type === 'book');
      return [
        { key: 'games', label: t('sectionGames'), color: '#2563eb', filterType: 'game', items: games },
        { key: 'artists', label: t('sectionArtists'), color: '#ea580c', filterType: 'music', items: artists },
        { key: 'tracks', label: t('sectionTracks'), color: '#0d9488', filterType: 'music', items: tracks },
        { key: 'screen', label: t('sectionScreen'), color: '#ca8a04', filterType: 'movie', items: screen },
      ];
    });

    const filteredInspectorRecs = computed(() => {
      const profileSet = new Set(profileSeeds.value.map(s => s.id));
      const selSet = new Set(selectedNodeIds.value);
      return inspectorRecs.value.filter(r => {
        if (profileSet.has(r.id) || selSet.has(r.id)) return false;
        if (!inspectorFilter.value) return true;
        if (inspectorFilter.value === 'music') return r.type === 'artist' || r.type === 'track';
        return r.type === inspectorFilter.value;
      });
    });

    const inspectedNodes = computed(() => {
      if (selectedNodeIds.value.length === 0) return [];
      const list = [];
      for (const nid of selectedNodeIds.value) {
        const node = fullServerGraph.nodes.find(n => n.id === nid);
        if (node) {
          list.push(node);
        }
      }
      return list;
    });

    // Filters
    const setDomainFilter = (flt) => {
      targetFilter.value = flt;
      computeRecommendations();
    };

    const setInspectorFilter = (flt) => {
      inspectorFilter.value = flt;
      fetchLiveInspectorRecommendations();
    };

    // Search Actions
    const clearSearch = () => {
      if (searchAbortController) {
        searchAbortController.abort();
      }
      searchQuery.value = '';
      searchResults.value = [];
      hasSearched.value = false;
      isSearching.value = false;
    };

    const executeSearch = async () => {
      const q = searchQuery.value.trim();
      if (!q) {
        searchResults.value = [];
        hasSearched.value = false;
        return;
      }
      if (searchAbortController) {
        searchAbortController.abort();
      }
      searchAbortController = new AbortController();
      isSearching.value = true;
      hasSearched.value = true;
      try {
        const res = await searchEntities(q, searchType.value, searchAbortController.signal);
        searchResults.value = Array.isArray(res) ? res : (res.results || []);
      } catch (err) {
        if (err.name !== 'AbortError') {
          console.error('Search error:', err);
        }
      } finally {
        isSearching.value = false;
      }
    };

    const onSearchTypeChange = () => {
      if (searchQuery.value.trim().length > 0) {
        executeSearch();
      }
    };

    // Seeds & Profile Actions
    const isNodeInProfile = (nodeId, nodeName) => {
      if (profileSeeds.value.some(s => s.id === nodeId)) return true;
      if (nodeName) {
        const clean = nodeName.trim().toLowerCase();
        return profileSeeds.value.some(s => s.name && s.name.trim().toLowerCase() === clean);
      }
      return false;
    };

    const isNodeInNegativeSeeds = (nodeId, nodeName) => {
      if (negativeSeeds.value.some(s => s.id === nodeId)) return true;
      if (nodeName) {
        const clean = nodeName.trim().toLowerCase();
        return negativeSeeds.value.some(s => s.name && s.name.trim().toLowerCase() === clean);
      }
      return false;
    };

    const addSeed = (item) => {
      if (!item || !item.id) return;
      // Remove from negative seeds if present so it flips to positive
      negativeSeeds.value = negativeSeeds.value.filter(s => s.id !== item.id);

      const cleanName = item.name ? item.name.trim().toLowerCase() : '';
      const cleanType = item.type || '';
      const alreadyInProfile = profileSeeds.value.some(s => {
        if (s.id === item.id) return true;
        if (cleanName && s.name && s.name.trim().toLowerCase() === cleanName && s.type === cleanType) {
          return true;
        }
        return false;
      });
      if (!alreadyInProfile) {
        profileSeeds.value.push(item);
        saveProfile();
        computeRecommendations();
      }
      clearSearch();
    };

    const addNegativeSeed = (item) => {
      if (!item || !item.id) return;
      // Remove from positive seeds if present so it flips to negative
      profileSeeds.value = profileSeeds.value.filter(s => s.id !== item.id);

      const cleanName = item.name ? item.name.trim().toLowerCase() : '';
      const cleanType = item.type || '';
      const alreadyIn = negativeSeeds.value.some(s => {
        if (s.id === item.id) return true;
        if (cleanName && s.name && s.name.trim().toLowerCase() === cleanName && s.type === cleanType) {
          return true;
        }
        return false;
      });

      if (!alreadyIn) {
        negativeSeeds.value.push({
          ...item,
          type: item.type || getRawType(item),
        });
        saveProfile();
        computeRecommendations();
      }
      clearSearch();
    };

    const addNodeToProfile = (node) => {
      if (!node || !node.id || isNodeInProfile(node.id, node.label)) return;
      const rawType = getRawType(node);
      addSeed({
        id: node.id,
        name: node.label,
        type: rawType,
        subtitle: node.desc || rawType,
      });
    };

    const addNodeToNegative = (node) => {
      if (!node || !node.id || isNodeInNegativeSeeds(node.id, node.label)) return;
      const rawType = getRawType(node);
      addNegativeSeed({
        id: node.id,
        name: node.label,
        type: rawType,
        subtitle: node.desc || rawType,
      });
    };

    const searchAndAddTag = async (tagStr) => {
      const existing = fullServerGraph.nodes.find(
        n => n.label && n.label.toLowerCase() === tagStr.toLowerCase()
      );
      if (existing) {
        addNodeToProfile(existing);
        return;
      }
      try {
        const clean = tagStr.replace(/^#/, '');
        const data = await searchEntities(clean, 'keyword');
        if (data && data.length > 0) {
          addSeed(data[0]);
        }
      } catch (e) {
        console.error('Failed to search and add tag:', e);
      }
    };

    const quickAdd = async (name) => {
      try {
        const data = await searchEntities(name);
        if (data && data.length > 0) {
          addSeed(data[0]);
        }
      } catch (e) {
        console.error('Quick add failed:', e);
      }
    };

    const removeSeed = (id) => {
      profileSeeds.value = profileSeeds.value.filter(s => s.id !== id);
      saveProfile();
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      } else {
        recommendations.value = [];
        fullServerGraph = { nodes: [], links: [] };
        if (constellation) {
          constellation.setGraphData(fullServerGraph, profileSeeds.value, {
            negativeSeeds: negativeSeeds.value,
          });
        }
      }
    };

    const removeNegativeSeed = (id) => {
      negativeSeeds.value = negativeSeeds.value.filter(s => s.id !== id);
      saveProfile();
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
    };

    const clearNegativeSeeds = () => {
      negativeSeeds.value = [];
      saveProfile();
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
    };

    const markAsKnown = (item) => {
      if (!item || !item.id) return;
      if (!knownItems.value.some(k => k.id === item.id)) {
        knownItems.value.push({
          id: item.id,
          name: item.title || item.name || item.id,
          type: item.type || getRawType(item),
          subtitle: item.subtitle || '',
        });
        saveProfile();
      }
      recommendations.value = recommendations.value.filter(r => r.id !== item.id);
      inspectorRecs.value = inspectorRecs.value.filter(r => r.id !== item.id);
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
    };

    const unmarkKnown = (id) => {
      knownItems.value = knownItems.value.filter(k => k.id !== id);
      saveProfile();
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
    };

    const clearKnownItems = () => {
      knownItems.value = [];
      saveProfile();
      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      }
    };

    const promoteKnownToSeed = (item) => {
      if (!item || !item.id) return;
      knownItems.value = knownItems.value.filter(k => k.id !== item.id);
      saveProfile();
      addSeed({
        id: item.id,
        name: item.name || item.title || item.id,
        type: item.type || getRawType(item),
        subtitle: item.subtitle || '',
      });
    };

    const clearProfile = () => {
      profileSeeds.value = [];
      negativeSeeds.value = [];
      recommendations.value = [];
      selectedNodeIds.value = [];
      inspectorRecs.value = [];
      sidebarMode.value = 'seeds';
      saveProfile();
      fullServerGraph = { nodes: [], links: [] };
      if (constellation) {
        constellation.setGraphData(fullServerGraph, profileSeeds.value, {
          negativeSeeds: negativeSeeds.value,
        });
        constellation.setSelectedNodes([]);
      }
    };

    // Recommendation & Diffusion Engine
    let inspectorAbortController = null;
    const fetchLiveInspectorRecommendations = async () => {
      if (selectedNodeIds.value.length === 0) {
        inspectorRecs.value = [];
        return;
      }
      if (inspectorAbortController) {
        inspectorAbortController.abort();
      }
      inspectorAbortController = new AbortController();
      isLoadingInspectorRecs.value = true;

      try {
        const data = await fetchRecommendations({
          seeds: selectedNodeIds.value,
          knownItems: knownItems.value.map(k => k.id),
          targetType: inspectorFilter.value,
          limit: 16,
          lang: currentLang.value,
          signal: inspectorAbortController.signal,
        });
        inspectorRecs.value = data.results || [];
      } catch (e) {
        if (e.name !== 'AbortError') {
          console.error('Failed to fetch inspector recommendations:', e);
        }
      } finally {
        isLoadingInspectorRecs.value = false;
      }
    };

    let recsAbortController = null;
    let recsDebounceTimer = null;

    const computeRecommendations = () => {
      if (profileSeeds.value.length === 0) {
        recommendations.value = [];
        fullServerGraph = { nodes: [], links: [] };
        if (constellation) {
          constellation.setGraphData(fullServerGraph, profileSeeds.value, {
            negativeSeeds: negativeSeeds.value,
          });
        }
        return;
      }

      clearTimeout(recsDebounceTimer);
      recsDebounceTimer = setTimeout(async () => {
        if (recsAbortController) {
          recsAbortController.abort();
        }
        recsAbortController = new AbortController();
        isLoadingRecs.value = true;

        try {
          const data = await fetchRecommendations({
            seeds: profileSeeds.value.map(s => s.id),
            negativeSeeds: negativeSeeds.value.map(s => s.id),
            knownItems: knownItems.value.map(k => k.id),
            beta: Number(negativeBeta.value) || 1.2,
            targetType: targetFilter.value,
            limit: 20,
            lang: currentLang.value,
            signal: recsAbortController.signal,
          });
          isOfflineCached.value = !!data.offlineCached;
          if (data.offline && (!data.results || data.results.length === 0)) {
            // Em offline sem cache, preserva visualização atual
            console.warn('[Offline] Sem dados em cache para esta combinação de sementes.');
          } else {
            recommendations.value = data.results || [];
            fullServerGraph = data.graph || { nodes: [], links: [] };
            if (constellation) {
              constellation.setGraphData(fullServerGraph, profileSeeds.value, {
                layoutMode: layoutMode.value,
                onlySharedBridges: onlySharedBridges.value,
                hideTrackNodes: hideTrackNodes.value,
                spacingScale: getSpacingScale(graphSpacing.value),
                negativeSeeds: negativeSeeds.value,
              });
            }
          }
        } catch (err) {
          if (err.name !== 'AbortError') {
            console.error('Falha ao calcular recomendações:', err);
          }
        } finally {
          isLoadingRecs.value = false;
        }
      }, 150);
    };

    const clearSelection = () => {
      selectedNodeIds.value = [];
      inspectorRecs.value = [];
      if (constellation) {
        constellation.setSelectedNodes([]);
      }
    };

    const focusNodeOnGraph = (nodeId) => {
      selectedNodeIds.value = [nodeId];
      sidebarMode.value = 'inspector';
      fetchLiveInspectorRecommendations();
      if (constellation) {
        constellation.setSelectedNodes([nodeId]);
        constellation.focusNode(nodeId);
      }
    };

    const setHoveredNode = (id) => {
      if (constellation) {
        constellation.setHoveredNode(id);
      }
    };

    const rebuildConstellation = () => {
      if (constellation) {
        constellation.setOptions({
          layoutMode: layoutMode.value,
          onlySharedBridges: onlySharedBridges.value,
          hideTrackNodes: hideTrackNodes.value,
          spacingScale: getSpacingScale(graphSpacing.value),
          negativeSeeds: negativeSeeds.value,
        });
      }
    };

    const fitGraphView = () => {
      if (constellation) {
        constellation.fitView();
      }
    };

    onMounted(() => {
      loadSavedProfile();

      const canvasEl = document.getElementById('constellation-canvas');
      constellation = new ConstellationEngine(canvasEl, {
        onNodeClick: (clicked, isMulti) => {
          if (isMulti) {
            const idx = selectedNodeIds.value.indexOf(clicked.id);
            if (idx >= 0) {
              selectedNodeIds.value.splice(idx, 1);
            } else {
              selectedNodeIds.value.push(clicked.id);
            }
          } else {
            selectedNodeIds.value = [clicked.id];
          }
          if (selectedNodeIds.value.length > 0) {
            sidebarMode.value = 'inspector';
            fetchLiveInspectorRecommendations();
          }
          if (constellation) {
            constellation.setSelectedNodes(selectedNodeIds.value);
          }
        },
        onNodeDoubleClick: (clicked) => {
          addNodeToProfile(clicked);
        },
        onHoverChange: (hoveredId) => {
          // Hover update handled inside engine
        },
        onBackgroundClick: () => {
          if (selectedNodeIds.value.length > 0) {
            clearSelection();
          }
        },
      });

      window.addEventListener('pointerdown', (e) => {
        const box = document.getElementById('search-box-wrapper');
        if (box && !box.contains(e.target)) {
          searchResults.value = [];
          hasSearched.value = false;
        }
      });

      window.addEventListener('online', () => {
        isOffline.value = false;
      });

      window.addEventListener('offline', () => {
        isOffline.value = true;
      });

      // Service Worker Registration for Offline Support & PWA
      if ('serviceWorker' in navigator) {
        navigator.serviceWorker.register('/sw.js', { scope: '/' })
          .then((registration) => {
            console.log('[SW] Service Worker registrado com sucesso no escopo:', registration.scope);
          })
          .catch((error) => {
            console.warn('[SW] Falha ao registrar Service Worker:', error);
          });
      }

      if (profileSeeds.value.length > 0) {
        computeRecommendations();
      } else {
        sidebarMode.value = 'seeds';
      }
    });

    return {
      currentLang,
      setLang,
      t,
      isOffline,
      isOfflineCached,
      sidebarMode,
      searchQuery,
      searchType,
      searchResults,
      isSearching,
      hasSearched,
      showLegend,
      profileSeeds,
      negativeSeeds,
      negativeBeta,
      knownItems,
      markAsKnown,
      unmarkKnown,
      clearKnownItems,
      promoteKnownToSeed,
      recommendations,
      isLoadingRecs,
      targetFilter,
      domainCounts,
      groupedRecommendations,
      setDomainFilter,
      executeSearch,
      clearSearch,
      onSearchTypeChange,
      isNodeInProfile,
      isNodeInNegativeSeeds,
      addSeed,
      addNegativeSeed,
      addNodeToProfile,
      addNodeToNegative,
      searchAndAddTag,
      quickAdd,
      removeSeed,
      removeNegativeSeed,
      clearNegativeSeeds,
      clearProfile,
      computeRecommendations,
      getSearchPlaceholder,
      getRawType,
      getTypeIcon,
      getSeedDotColor,
      getTypeBadgeClass,
      getAffinityLabel: (score) => getAffinityLabel(score, t),
      isBridgeReason: (reason) => reason && (
        reason.includes('Ponte') ||
        reason.includes('Bridge') ||
        reason.includes('Confluência') ||
        reason.includes('Co-escuta') ||
        reason.includes('Co-listening') ||
        reason.includes('Co-jogabilidade') ||
        reason.includes('co-play') ||
        reason.includes('Adaptação') ||
        reason.includes('Adapted') ||
        reason.includes('Trilha sonora composta') ||
        reason.includes('Soundtrack composed')
      ),
      selectedNodeIds,
      setHoveredNode,
      inspectedNodes,
      inspectorRecs,
      inspectorFilter,
      filteredInspectorRecs,
      isLoadingInspectorRecs,
      setInspectorFilter,
      clearSelection,
      focusNodeOnGraph,
      onlySharedBridges,
      hideTrackNodes,
      layoutMode,
      setLayoutMode,
      graphSpacing,
      setGraphSpacing,
      rebuildConstellation,
      fitGraphView,
    };
  }
}).mount('#app');

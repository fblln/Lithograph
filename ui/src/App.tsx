import { useCallback, useEffect, useLayoutEffect, useMemo, useState } from 'react'
import { getGraphLayout } from './api/graph'
import { getNodeDetail, type NodeDetail } from './api/nodeDetail'
import { getArchitectureSummary, type ArchitectureCluster, type ArchitectureSummary } from './api/architecture'
import { RpcError, setActiveProjectId } from './api/rpc'
import { listProjects, type ProjectMetadata } from './api/projects'
import type { LayoutEdge, LayoutResult, PositionedNode } from './graph/types'
import { GraphScene } from './graph/GraphScene'
import { useDragPositions } from './graph/useDragPositions'
import { parseUrlState, serializeUrlState } from './urlState'
import { TopBar } from './components/TopBar'
import { Sidebar, type SidebarTab } from './components/Sidebar'
import { DetailPanel } from './components/DetailPanel'
import type { ViewMode } from './components/ViewModeToggle'
import { GraphToolbar, type EdgeView, type OverlayMode } from './components/GraphToolbar'
import { TensionRail } from './components/TensionRail'
import { StatusBanner } from './components/StatusBanner'
import { ClusterTensionDrilldown } from './components/ClusterTensionDrilldown'
import { WorkspaceShell } from './components/WorkspaceShell'
import type { InvestigationQueryState } from './investigations'
import { getGraphAnalytics, type AnalyticsNode, type HealthFinding } from './api/analytics'
import { getRepositoryTensions, type RepositoryTension } from './api/tensions'
import { resolveTagExpression } from './api/tags'
import { DocsWorkspace } from './components/DocsWorkspace'
import { ClusterCouplingMatrix } from './components/ClusterCouplingMatrix'
import type { RepositoryArea } from './architectureOverview'
import type { VisualCluster } from './graph/clusterLayout'
import { humanClusterNameFromEvidence } from './clusterIdentity'
import { deriveClusterIdentities } from './clusterIdentity'
import { computeClusterLayout } from './graph/clusterLayout'
import { ClusterRelationshipInspector } from './components/ClusterRelationshipInspector'
import { filterVisibleEdges } from './graph/edgeProvenance'
import { deriveDisplayRootPrefix } from './displayRoot'
import { deriveSliceFacetCounts } from './graph/sliceFacets'

type Status = 'loading' | 'ready' | 'error'
interface NavigationContext {
  label: string
  centerNode?: string
  scopeNodeIds: string[]
  cluster?: ArchitectureCluster
  viewMode?: ViewMode
  overlayMode?: OverlayMode
  edgeView?: EdgeView
  edgeKinds?: string[]
  zoom?: number
  maxNodes?: number | null
  maxEdges?: number | null
  activeLabels?: string[]
  tagExpression?: string
  tagNodeIds?: string[]
  selectedNodeId?: string
  tensionId?: string
  showUnprovenEdges?: boolean
}

function ExplorerApp({ projects, projectId, onProjectChange }: { projects: ProjectMetadata[]; projectId: string; onProjectChange: (id: string) => void }) {
  // Capture the incoming URL once. Subsequent URL writes are an output of the
  // current interaction, not an instruction to reset the live explorer state.
  const [initialUrlState] = useState(() => parseUrlState(window.location.search))
  const [layout, setLayout] = useState<LayoutResult | null>(null)
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState<string | null>(null)
  const [centerNode, setCenterNode] = useState<string | undefined>(initialUrlState.centerNode)
  const [selected, setSelected] = useState<PositionedNode | null>(null)
  const [pendingSelectedId, setPendingSelectedId] = useState<string | undefined>(initialUrlState.selectedNode)
  const [detail, setDetail] = useState<NodeDetail | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)
  const [activeLabels, setActiveLabels] = useState<Set<string>>(new Set(initialUrlState.nodeLabels))
  const [viewMode, setViewMode] = useState<ViewMode>(initialUrlState.viewMode)
  const [maxNodes, setMaxNodes] = useState<number | undefined>(initialUrlState.maxNodes)
  const [maxEdges, setMaxEdges] = useState<number | undefined>(initialUrlState.maxEdges)
  const [clusters, setClusters] = useState<ArchitectureCluster[]>([])
  const [architecture, setArchitecture] = useState<ArchitectureSummary>({ clusters: [], entry_points: [], hotspots: [] })
  const [metricValues, setMetricValues] = useState<Map<string, number>>(new Map())
  const [analyticsNodes, setAnalyticsNodes] = useState<AnalyticsNode[]>([])
  const [repositoryTensions, setRepositoryTensions] = useState<RepositoryTension[]>([])
  const [overlayMode, setOverlayMode] = useState<OverlayMode>('kind')
  const [edgeView, setEdgeView] = useState<EdgeView>('nodes')
  const [edgeKinds, setEdgeKinds] = useState<Set<string>>(new Set())
  const [showUnprovenEdges, setShowUnprovenEdges] = useState(initialUrlState.showUnprovenEdges ?? true)
  const [zoom, setZoom] = useState(1)
  const [queryState, setQueryState] = useState<InvestigationQueryState>({ query: 'MATCH (a:Artifact)-[:Contains]->(b:Symbol) RETURN a, b', rows: [] })
  const [healthFindings, setHealthFindings] = useState<HealthFinding[]>([])
  const [selectedTension, setSelectedTension] = useState<RepositoryTension | undefined>()
  const [requestedTensionId, setRequestedTensionId] = useState<string | undefined>(initialUrlState.tensionId)
  const [requestedSidebarTab, setRequestedSidebarTab] = useState<SidebarTab | undefined>()
  const [tagExpression, setTagExpression] = useState(initialUrlState.tagExpression ?? '')
  const [tagNodeIds, setTagNodeIds] = useState<string[]>([])
  const [workspaceMode, setWorkspaceMode] = useState<'explore' | 'docs'>(initialUrlState.workspaceMode ?? 'explore')
  const [docSectionId, setDocSectionId] = useState<string | undefined>(initialUrlState.docSectionId)
  const [relatedDocEntityId, setRelatedDocEntityId] = useState<string | undefined>(initialUrlState.selectedNode ?? initialUrlState.tensionId ?? initialUrlState.centerNode)
  const [scopedCluster, setScopedCluster] = useState<ArchitectureCluster | undefined>()
  const [selectedVisualClusterId, setSelectedVisualClusterId] = useState<string | null>(null)
  const [matrixScopeNodeIds, setMatrixScopeNodeIds] = useState<string[]>([])
  const [interClusterOnly, setInterClusterOnly] = useState(false)
  const [navigation, setNavigation] = useState<NavigationContext[]>(() => [
    { label: 'Overview', scopeNodeIds: [] },
    ...(initialUrlState.centerNode ? [{ label: shortNodeLabel(initialUrlState.centerNode), centerNode: initialUrlState.centerNode, scopeNodeIds: [] }] : []),
  ])
  const effectiveScopeNodeIds = useMemo(() => {
    const clusterIds = scopedCluster?.members ?? matrixScopeNodeIds
    if (tagNodeIds.length === 0) return clusterIds
    if (clusterIds.length === 0) return tagNodeIds
    const allowed = new Set(clusterIds)
    return tagNodeIds.filter((id) => allowed.has(id))
  }, [matrixScopeNodeIds, scopedCluster, tagNodeIds])
  const documentationScopeNodeIds = effectiveScopeNodeIds.length > 0 ? effectiveScopeNodeIds : layout?.nodes.map((node) => node.id) ?? []
  const displayRootPrefix = useMemo(() => deriveDisplayRootPrefix(layout?.nodes ?? []), [layout?.nodes])
  const sliceFacetCounts = useMemo(() => layout ? deriveSliceFacetCounts(layout) : { nodeLabels: new Map<string, number>(), edgeKinds: new Map<string, number>() }, [layout])
  const availableEdgeKinds = useMemo(() => [...sliceFacetCounts.edgeKinds.keys()], [sliceFacetCounts])
  const visibleLayoutEdges = useMemo(() => !layout ? [] : filterVisibleEdges(layout.edges, edgeKinds, showUnprovenEdges), [edgeKinds, layout, showUnprovenEdges])
  const completeClusterLinks = useMemo(() => (architecture.cluster_links ?? []).map((link) => {
    const underlying = filterVisibleEdges(link.underlying, edgeKinds, showUnprovenEdges)
    const counts = new Map<string, number>()
    for (const edge of underlying) counts.set(edge.kind, (counts.get(edge.kind) ?? 0) + 1)
    return { ...link, count: underlying.length, underlying, kinds: [...counts].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind)) }
  }).filter((link) => link.count > 0), [architecture.cluster_links, edgeKinds, showUnprovenEdges])
  const visualClusterLayout = useMemo(() => layout ? computeClusterLayout(layout.nodes, clusters, visibleLayoutEdges, completeClusterLinks) : null, [clusters, completeClusterLinks, layout, visibleLayoutEdges])
  const visualClusterIdentities = useMemo(() => visualClusterLayout && layout ? deriveClusterIdentities(visualClusterLayout.clusters, layout.nodes, visualClusterLayout.links, architecture.entry_points, repositoryTensions) : new Map(), [architecture.entry_points, layout, repositoryTensions, visualClusterLayout])
  const navigationSnapshot = useMemo(() => ({ viewMode, overlayMode, edgeView, edgeKinds: [...edgeKinds], showUnprovenEdges, zoom, maxNodes: maxNodes ?? null, maxEdges: maxEdges ?? null, activeLabels: [...activeLabels], tagExpression, tagNodeIds: [...tagNodeIds], selectedNodeId: selected?.id, tensionId: selectedTension?.id }), [activeLabels, edgeKinds, edgeView, maxEdges, maxNodes, overlayMode, selected?.id, selectedTension?.id, showUnprovenEdges, tagExpression, tagNodeIds, viewMode, zoom])
  const dragPositions = useDragPositions(layout?.graph_snapshot_id ?? '')
  const overlayValues = useMemo(() => {
    if (!layout || overlayMode === 'kind') return new Map<string, number>()
    if (overlayMode === 'centrality') {
      if (metricValues.size > 0) return metricValues
      const maximum = Math.max(...analyticsNodes.map((node) => node.page_rank), 1)
      return new Map(analyticsNodes.map((node) => [node.id, node.page_rank / maximum]))
    }
    if (overlayMode === 'tension') {
      const severity = { low: 0.35, medium: 0.6, high: 0.82, critical: 1 } as const
      const values = new Map<string, number>()
      const tensions = selectedTension ? [selectedTension] : repositoryTensions
      for (const tension of tensions) {
        const value = severity[tension.severity.toLowerCase() as keyof typeof severity] ?? 0.5
        for (const id of tension.affected_nodes) values.set(id, Math.max(values.get(id) ?? 0, value))
      }
      return values
    }
    if (!selected) return new Map<string, number>()
    const adjacency = new Map<string, string[]>()
    for (const edge of layout.edges) {
      adjacency.set(edge.source, [...(adjacency.get(edge.source) ?? []), edge.target])
      adjacency.set(edge.target, [...(adjacency.get(edge.target) ?? []), edge.source])
    }
    const distances = new Map<string, number>([[selected.id, 0]])
    let frontier = [selected.id]
    for (let hop = 1; hop <= 3; hop += 1) {
      const next: string[] = []
      for (const id of frontier) for (const neighbor of adjacency.get(id) ?? []) {
        if (distances.has(neighbor)) continue
        distances.set(neighbor, hop)
        next.push(neighbor)
      }
      frontier = next
    }
    return new Map([...distances].map(([id, hop]) => [id, 1 - hop * 0.22]))
  }, [analyticsNodes, layout, metricValues, overlayMode, repositoryTensions, selected, selectedTension])

  useEffect(() => {
    // A fast filter/focus sequence can leave older layout responses in
    // flight. Aborting the obsolete request ensures the canvas only receives
    // the newest graph slice rather than briefly snapping back to old data.
    const controller = new AbortController()
    setStatus('loading')
    getGraphLayout({ center_node: centerNode, node_labels: [...activeLabels], node_ids: effectiveScopeNodeIds, max_nodes: maxNodes, max_edges: maxEdges }, controller.signal)
      .then((result) => {
        if (controller.signal.aborted) return
        setLayout(result)
        setStatus('ready')
        setError(null)
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return
        setStatus('error')
        setError(cause instanceof RpcError ? cause.message : String(cause))
      })
    return () => controller.abort()
  }, [centerNode, activeLabels, effectiveScopeNodeIds, maxEdges, maxNodes])

  useEffect(() => {
    if (!initialUrlState.tagExpression) return
    resolveTagExpression(initialUrlState.tagExpression).then(setTagNodeIds, () => setTagNodeIds([]))
  }, [initialUrlState.tagExpression])

  // Keeps the URL a shareable/bookmarkable snapshot of the current view
  // (LIT-24.17 AC1). `replaceState` rather than `pushState`: every field
  // here already has its own undo path in the UI (Clear, view toggle,
  // budget field), so it isn't worth spamming browser history on top of
  // that.
  useEffect(() => {
    const query = serializeUrlState({ projectId, centerNode, viewMode, maxNodes, maxEdges, nodeLabels: [...activeLabels], selectedNode: selected?.id, tensionId: requestedTensionId, tagExpression, workspaceMode, docSectionId, showUnprovenEdges })
    window.history.replaceState(null, '', `${window.location.pathname}${query}`)
  }, [projectId, centerNode, viewMode, maxEdges, maxNodes, activeLabels, selected, requestedTensionId, tagExpression, workspaceMode, docSectionId, showUnprovenEdges])

  useEffect(() => {
    if (!layout || !pendingSelectedId) return
    setSelected(layout.nodes.find((node) => node.id === pendingSelectedId) ?? null)
    setPendingSelectedId(undefined)
  }, [layout, pendingSelectedId])

  // Architecture clusters are a whole-graph property, not scoped to the
  // current focused/budgeted layout slice, so they're fetched once
  // independently of `load` rather than re-fetched on every layout change.
  useEffect(() => {
    getArchitectureSummary()
      .then((summary) => { setArchitecture(summary); setClusters(summary.clusters) })
      .catch(() => { setArchitecture({ clusters: [], entry_points: [], hotspots: [] }); setClusters([]) })
  }, [])

  useEffect(() => {
    getGraphAnalytics()
      .then((result) => { setAnalyticsNodes(Array.isArray(result.nodes) ? result.nodes : []); setHealthFindings(Array.isArray(result.findings) ? result.findings : []) })
      .catch(() => { setAnalyticsNodes([]); setHealthFindings([]) })
    getRepositoryTensions().then(setRepositoryTensions, () => setRepositoryTensions([]))
  }, [])

  useEffect(() => {
    if (selected === null) {
      setDetail(null)
      setDetailError(null)
      return
    }
    let current = true
    setDetail(null)
    setDetailError(null)
    getNodeDetail(selected.id).then(
      (result) => current && setDetail(result),
      (cause: unknown) => current && setDetailError(cause instanceof RpcError ? cause.message : String(cause)),
    )
    return () => {
      current = false
    }
  }, [selected])

  function handleSelect(node: PositionedNode) {
    setSelected(node)
    setSelectedVisualClusterId(null)
    setRelatedDocEntityId(node.id)
  }

  function handleFocus(node: PositionedNode) {
    handleFocusId(node.id)
  }

  const handleFocusId = useCallback((id: string) => {
    const node = layout?.nodes.find((candidate) => candidate.id === id)
    const scopeNodeIds = scopedCluster?.members ?? matrixScopeNodeIds
    setNavigation((previous) => previous.at(-1)?.centerNode === id ? previous : pushNavigation(previous, { label: node?.name ?? shortNodeLabel(id), centerNode: id, scopeNodeIds, cluster: scopedCluster }, navigationSnapshot))
    setCenterNode(id)
    setSelected(null)
    setSelectedVisualClusterId(null)
    setRelatedDocEntityId(id)
  }, [layout, matrixScopeNodeIds, navigationSnapshot, scopedCluster])

  const applyNavigationContext = useCallback((context: NavigationContext) => {
    setCenterNode(context.centerNode)
    setMatrixScopeNodeIds(context.cluster ? [] : context.scopeNodeIds)
    setScopedCluster(context.cluster)
    setViewMode(context.viewMode ?? viewMode)
    setOverlayMode(context.overlayMode ?? overlayMode)
    setEdgeView(context.edgeView ?? edgeView)
    if (context.edgeKinds) setEdgeKinds(new Set(context.edgeKinds))
    if (context.showUnprovenEdges !== undefined) setShowUnprovenEdges(context.showUnprovenEdges)
    setZoom(context.zoom ?? zoom)
    if ('maxNodes' in context) setMaxNodes(context.maxNodes ?? undefined)
    if ('maxEdges' in context) setMaxEdges(context.maxEdges ?? undefined)
    if (context.activeLabels) setActiveLabels(new Set(context.activeLabels))
    if (context.tagExpression !== undefined) setTagExpression(context.tagExpression)
    if (context.tagNodeIds) setTagNodeIds(context.tagNodeIds)
    setSelected(context.selectedNodeId ? layout?.nodes.find((node) => node.id === context.selectedNodeId) ?? null : null)
    setPendingSelectedId(context.selectedNodeId)
    const tension = context.tensionId ? repositoryTensions.find((candidate) => candidate.id === context.tensionId) : undefined
    setSelectedTension(tension)
    setRequestedTensionId(tension?.id)
    setRelatedDocEntityId(context.centerNode ?? context.cluster?.id)
  }, [edgeView, layout?.nodes, overlayMode, repositoryTensions, viewMode, zoom])

  function navigateToArea(area: RepositoryArea) {
    const context = { label: area.name, scopeNodeIds: area.nodeIds }
    setNavigation((previous) => pushNavigation(previous, context, navigationSnapshot))
    applyNavigationContext(context)
  }

  function navigateToCluster(cluster?: ArchitectureCluster) {
    if (!cluster) { navigateToBreadcrumb(0); return }
    setMaxNodes((previous) => Math.max(previous ?? 150, cluster.members.length))
    const context = { label: humanClusterNameFromEvidence(cluster), scopeNodeIds: cluster.members, cluster }
    setNavigation((previous) => pushNavigation(previous, context, navigationSnapshot))
    applyNavigationContext(context)
    setSelectedVisualClusterId(cluster.id)
    setRelatedDocEntityId(cluster.id)
  }

  function navigateToVisualCluster(cluster: VisualCluster) {
    setSelectedVisualClusterId(cluster.id)
    setSelected(null)
    setRelatedDocEntityId(cluster.id)
    if (cluster.analyticalCluster) {
      navigateToCluster(cluster.analyticalCluster)
      return
    }
    setMaxNodes((previous) => Math.max(previous ?? 150, cluster.members.length))
    const context = {
      label: humanClusterNameFromEvidence(cluster),
      scopeNodeIds: cluster.members,
    }
    setNavigation((previous) => pushNavigation(previous, context, navigationSnapshot))
    applyNavigationContext(context)
  }

  function navigateToBreadcrumb(index: number) {
    const context = navigation[index]
    if (!context) return
    setNavigation((previous) => previous.slice(0, index + 1))
    applyNavigationContext(context)
    setSelectedVisualClusterId(context.cluster?.id ?? null)
  }

  function navigateToRelationship(edge: LayoutEdge) {
    const context = {
      label: `${humanRelationKind(edge.kind)} relationship`,
      centerNode: edge.source,
      scopeNodeIds: [...new Set([edge.source, edge.target])],
    }
    setNavigation((previous) => pushNavigation(previous, context, navigationSnapshot))
    applyNavigationContext(context)
    setSelectedVisualClusterId(null)
    setEdgeView('nodes')
    setPendingSelectedId(edge.source)
    setRelatedDocEntityId(`${edge.source} → ${edge.target}`)
  }

  useEffect(() => {
    function restorePreviousContext(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      const target = event.target
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return
      if (selected) {
        setSelected(null)
        return
      }
      if (selectedVisualClusterId && navigation.length === 1) {
        setSelectedVisualClusterId(null)
        return
      }
      if (navigation.length > 1) {
        const context = navigation[navigation.length - 2]
        setNavigation((previous) => previous.slice(0, -1))
        applyNavigationContext(context)
        setSelectedVisualClusterId(context.cluster?.id ?? null)
      }
    }
    window.addEventListener('keydown', restorePreviousContext)
    return () => window.removeEventListener('keydown', restorePreviousContext)
  }, [applyNavigationContext, navigation, selected, selectedVisualClusterId])

  function handleToggleLabel(label: string) {
    setActiveLabels((previous) => {
      const next = new Set(previous)
      if (next.has(label)) next.delete(label)
      else next.add(label)
      return next
    })
  }

  function toggleEdgeKind(kind: string) {
    setEdgeKinds((previous) => {
      const next = previous.size === 0 ? new Set(availableEdgeKinds) : new Set(previous)
      if (next.has(kind)) next.delete(kind)
      else next.add(kind)
      return next.size === availableEdgeKinds.length ? new Set() : next
    })
  }

  const handleSemanticLabels = useCallback((labels: string[]) => {
    setActiveLabels(new Set(labels))
  }, [])

  return (
    <WorkspaceShell
      topBar={<TopBar projects={projects} projectId={projectId} onProjectChange={onProjectChange} centerLabel={navigation.at(-1)?.label ?? 'Overview'} breadcrumbs={navigation.map((context) => context.label)} displayRootPrefix={displayRootPrefix} onNavigateBreadcrumb={navigateToBreadcrumb} onBack={() => navigateToBreadcrumb(Math.max(0, navigation.length - 2))} status={status} onFocus={handleFocusId} clusters={clusters} onFocusCluster={navigateToCluster} onSelectTension={(tension) => { setSelectedTension(tension); setOverlayMode('tension'); setRequestedTensionId(tension.id); if (tension.affected_nodes[0]) handleFocusId(tension.affected_nodes[0]); setRelatedDocEntityId(tension.id) }} snapshotId={layout?.graph_snapshot_id} renderedNodes={layout?.budget.nodes_returned} availableNodes={layout?.budget.nodes_available} scopeNodeIds={effectiveScopeNodeIds} workspaceMode={workspaceMode} onWorkspaceMode={setWorkspaceMode} />}
      sidebar={layout && (
          <Sidebar
            layout={layout}
            nodeLabelCounts={sliceFacetCounts.nodeLabels}
            activeLabels={activeLabels}
            onToggleLabel={handleToggleLabel}
            maxNodes={maxNodes}
            onApplyMaxNodes={setMaxNodes}
            maxEdges={maxEdges}
            onApplyMaxEdges={setMaxEdges}
            onFocusNode={handleFocusId}
            onMetricValues={(values) => { setMetricValues(values); setOverlayMode('centrality') }}
            onSemanticLabels={handleSemanticLabels}
            queryState={queryState}
            onQueryStateChange={setQueryState}
            requestedTab={requestedSidebarTab}
            tagExpression={tagExpression}
            onTagExpressionChange={(expression, nodeIds) => { setTagExpression(expression); setTagNodeIds(nodeIds); setCenterNode(undefined); setSelected(null) }}
            clusters={clusters}
            architecture={architecture}
            tensions={repositoryTensions}
            scopedClusterId={scopedCluster?.id}
            interClusterOnly={interClusterOnly}
            onAreaScope={navigateToArea}
            onClearArchitectureScope={() => navigateToBreadcrumb(0)}
            onSelectTension={(tension) => { setSelectedTension(tension); setOverlayMode('tension'); setRequestedTensionId(tension.id); setRelatedDocEntityId(tension.id); if (tension.affected_nodes[0]) handleFocusId(tension.affected_nodes[0]) }}
            onClusterScope={navigateToCluster}
            onInterClusterOnly={setInterClusterOnly}
            onRelatedEntity={setRelatedDocEntityId}
            scopeNodeIds={effectiveScopeNodeIds}
            investigationState={{ version: 1, graphSnapshotId: layout.graph_snapshot_id, urlState: { projectId, centerNode, viewMode, maxNodes, maxEdges, tensionId: requestedTensionId, tagExpression, workspaceMode, docSectionId, showUnprovenEdges }, selectedNodeId: selected?.id, activeLabels: [...activeLabels], metricValues: [...metricValues], queryState, focusedSubgraph: { center_node: layout.center_node, nodes: layout.nodes, edges: layout.edges, budget: layout.budget }, healthFindings, selectedTension, overlayMode, edgeView }}
            onRestoreInvestigation={(item) => {
              setCenterNode(item.urlState.centerNode)
              setNavigation([{ label: 'Overview', scopeNodeIds: [] }, ...(item.urlState.centerNode ? [{ label: shortNodeLabel(item.urlState.centerNode), centerNode: item.urlState.centerNode, scopeNodeIds: [] }] : [])])
              setViewMode(item.urlState.viewMode)
              setMaxNodes(item.urlState.maxNodes)
              setMaxEdges(item.urlState.maxEdges)
              setActiveLabels(new Set(item.activeLabels))
              // The current layout is still usable while a restored focus is
              // loading. Re-select immediately when possible and remember
              // the ID for the incoming focused layout if it is not.
              setSelected(layout.nodes.find((node) => node.id === item.selectedNodeId) ?? null)
              setPendingSelectedId(item.selectedNodeId)
              setMetricValues(new Map(item.metricValues ?? []))
              setOverlayMode(item.overlayMode ?? ((item.metricValues?.length ?? 0) > 0 ? 'centrality' : 'kind'))
              setEdgeView(item.edgeView ?? 'nodes')
              setShowUnprovenEdges(item.urlState.showUnprovenEdges ?? true)
              setQueryState(item.queryState ?? { query: 'MATCH (a:Artifact)-[:Contains]->(b:Symbol) RETURN a, b', rows: [] })
              setSelectedTension(item.selectedTension)
              setRequestedTensionId(item.selectedTension?.id ?? item.urlState.tensionId)
              setRelatedDocEntityId(item.selectedNodeId ?? item.selectedTension?.id ?? item.urlState.centerNode)
              setTagExpression(item.urlState.tagExpression ?? '')
              if (item.urlState.tagExpression) resolveTagExpression(item.urlState.tagExpression).then(setTagNodeIds, () => setTagNodeIds([]))
              else setTagNodeIds([])
              setWorkspaceMode(item.urlState.workspaceMode ?? 'explore')
              setDocSectionId(item.urlState.docSectionId)
            }}
          />
      )}
      inspector={workspaceMode === 'docs'
        ? <DocsWorkspace currentSnapshotId={layout?.graph_snapshot_id} relatedEntityId={relatedDocEntityId} selectedSectionId={docSectionId} agentContext={{ scopeId: scopedCluster?.id ?? relatedDocEntityId ?? 'current-view', nodeIds: documentationScopeNodeIds, edgeCount: layout?.edges.length ?? 0, evidenceCount: detail?.evidence.length ?? 0, tensionCount: selectedTension ? 1 : 0, graphSnapshotId: layout?.graph_snapshot_id }} onSelectSection={setDocSectionId} onFocus={handleFocusId} onTagScope={(tag, nodeIds) => { setTagExpression(tag); setTagNodeIds(nodeIds); setWorkspaceMode('explore'); setCenterNode(undefined); setSelected(null) }} />
        : selected ? <DetailPanel
            node={selected}
            detail={detail}
            detailError={detailError}
            displayRootPrefix={displayRootPrefix}
            onFocus={handleFocus}
            onClear={() => setSelected(null)}
          /> : undefined}
    >
          <div data-testid="view-mode-control" className="contents top-3">
            <GraphToolbar viewMode={viewMode} overlayMode={overlayMode} edgeView={edgeView} zoom={zoom} layoutCustomized={layout?.nodes.some((node) => dragPositions.hasOverride(node.id)) ?? false} truncated={Boolean(layout?.budget.nodes_truncated || layout?.budget.edges_truncated)} edgeCountsTruncated={layout?.budget.edges_truncated ?? false} omittedNodes={layout ? Math.max(0, layout.budget.nodes_available - layout.budget.nodes_returned) : 0} omittedEdges={layout ? Math.max(0, layout.budget.edges_available - layout.budget.edges_returned) : 0} availableEdgeKinds={availableEdgeKinds} edgeKindCounts={sliceFacetCounts.edgeKinds} activeEdgeKinds={edgeKinds} showUnprovenEdges={showUnprovenEdges} onViewMode={setViewMode} onOverlayMode={setOverlayMode} onEdgeView={setEdgeView} onToggleEdgeKind={toggleEdgeKind} onShowUnprovenEdges={setShowUnprovenEdges} onResetLayout={dragPositions.clearAll} onZoom={setZoom} onRaiseBudget={() => { if (!layout) return; setMaxNodes(Math.min(layout.budget.nodes_available, Math.max(layout.budget.nodes_returned * 2, 300))); setMaxEdges(Math.min(layout.budget.edges_available, Math.max(layout.budget.edges_returned * 2, 800))) }} />
          </div>
          {status === 'error' && (
            <div data-testid="graph-error" className="absolute inset-x-0 top-14 mx-auto w-fit"><StatusBanner>{error}</StatusBanner></div>
          )}
          {layout && viewMode === 'matrix' && <ClusterCouplingMatrix clusters={clusters} edges={layout.edges} onInspect={(source, target) => { setMatrixScopeNodeIds([...new Set([...source.members, ...target.members])]); setScopedCluster(undefined); setInterClusterOnly(source.id !== target.id); setEdgeView('nodes'); setViewMode('cluster'); setRelatedDocEntityId(source.id === target.id ? source.id : `${source.id} → ${target.id}`) }} />}
          {layout && viewMode !== 'matrix' && (
            <GraphScene
              layout={layout}
              viewMode={viewMode}
              clusters={clusters}
              selectedId={selected?.id ?? null}
              onSelect={handleSelect}
              dragPositions={dragPositions}
              metricValues={overlayValues}
              interClusterOnly={interClusterOnly}
              edgeView={edgeView}
              zoom={zoom}
              entryPoints={architecture.entry_points}
              tensions={repositoryTensions}
              selectedClusterId={selectedVisualClusterId}
              onSelectCluster={(cluster) => { setSelectedVisualClusterId(cluster.id); setSelected(null); setRelatedDocEntityId(cluster.id) }}
              onEnterCluster={navigateToVisualCluster}
              edgeKinds={edgeKinds}
              showUnprovenEdges={showUnprovenEdges}
              clusterLinks={architecture.cluster_links}
              onFocusNode={handleFocus}
            />
          )}
          {layout && layout.nodes.length === 0 && <div role="status" className="absolute inset-0 grid place-items-center p-6 text-center"><div><h1 className="text-sm font-semibold" style={{ color: 'var(--atlas-text-bright)' }}>No graph nodes yet</h1><p className="mt-1 text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>Run <code>lithograph init</code> for this repository, then refresh the explorer.</p></div></div>}
          {visualClusterLayout && viewMode === 'cluster' && <ClusterRelationshipInspector clusterLayout={visualClusterLayout} identities={visualClusterIdentities} selectedClusterId={selectedVisualClusterId} onInspectRelationship={navigateToRelationship} />}
          {(overlayMode === 'tension' || navigation.length > 1 || selectedTension) && <TensionRail onFocus={handleFocusId} onInspect={(id) => { handleFocusId(id); setPendingSelectedId(id) }} onUseQuery={(query) => { setQueryState({ query, rows: [] }); setRequestedSidebarTab('query') }} requestedTensionId={requestedTensionId} onSelectTension={(tension) => { setSelectedTension(tension); setOverlayMode('tension'); setRequestedTensionId(tension.id); setRelatedDocEntityId(tension.id) }} scopeNodeIds={effectiveScopeNodeIds} repositoryTensions={repositoryTensions} />}
          {scopedCluster && <ClusterTensionDrilldown clusters={[scopedCluster]} onFocus={handleFocusId} />}
    </WorkspaceShell>
  )
}

function App() {
  const [requestedProject] = useState(() => parseUrlState(window.location.search).projectId)
  const [projects, setProjects] = useState<ProjectMetadata[] | null>(null)
  const [projectId, setProjectId] = useState(requestedProject ?? 'primary')

  useEffect(() => {
    const controller = new AbortController()
    listProjects(controller.signal)
      .then((available) => {
        setProjects(available)
        if (!available.some((project) => project.id === requestedProject)) {
          setProjectId(available.find((project) => project.is_primary)?.id ?? 'primary')
        }
      })
      .catch(() => setProjects([{ id: 'primary', name: 'Primary project', is_primary: true }]))
    return () => controller.abort()
  }, [requestedProject])

  useLayoutEffect(() => setActiveProjectId(projectId), [projectId])
  if (!projects) return <div role="status">Loading projects…</div>
  const switchProject = (next: string) => {
    if (next === projectId || !projects.some((project) => project.id === next)) return
    setActiveProjectId(next)
    const query = serializeUrlState({ projectId: next, viewMode: 'cluster' })
    window.history.replaceState(null, '', `${window.location.pathname}${query}`)
    setProjectId(next)
  }
  return <ExplorerApp key={projectId} projects={projects} projectId={projectId} onProjectChange={switchProject} />
}

function shortNodeLabel(id: string): string {
  return id.split(/[/:#]/).filter(Boolean).at(-1) ?? id
}

function humanRelationKind(value: string): string {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[-_]+/g, ' ')
}

function pushNavigation(previous: NavigationContext[], next: NavigationContext, snapshot: Omit<NavigationContext, 'label' | 'scopeNodeIds' | 'centerNode' | 'cluster'>): NavigationContext[] {
  const current = previous.at(-1)
  if (!current) return [next]
  return [...previous.slice(0, -1), { ...current, ...snapshot }, next]
}

export default App

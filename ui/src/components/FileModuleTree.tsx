import { useMemo, useState } from 'react'
import type { PositionedNode } from '../graph/types'

interface Folder {
  folders: Map<string, Folder>
  files: FileEntry[]
}

interface FileEntry {
  path: string
  focusNode: PositionedNode
  matches: PositionedNode[]
}

const LABEL_PRIORITY = ['Artifact', 'Module', 'Documentation', 'Config', 'Command', 'Symbol']

function fileEntries(nodes: PositionedNode[], query: string): FileEntry[] {
  const needle = query.trim().toLocaleLowerCase()
  const byPath = new Map<string, PositionedNode[]>()
  for (const node of nodes) {
    if (!node.file_path) continue
    if (needle && !`${node.file_path} ${node.name} ${node.label} ${node.id}`.toLocaleLowerCase().includes(needle)) continue
    const matches = byPath.get(node.file_path) ?? []
    matches.push(node)
    byPath.set(node.file_path, matches)
  }
  return [...byPath].map(([path, matches]) => ({
    path,
    matches,
    focusNode: [...matches].sort((a, b) => labelRank(a.label) - labelRank(b.label) || a.id.localeCompare(b.id))[0],
  }))
}

function labelRank(label: string) {
  const index = LABEL_PRIORITY.indexOf(label)
  return index === -1 ? LABEL_PRIORITY.length : index
}

function buildTree(files: FileEntry[]): Folder {
  const root: Folder = { folders: new Map(), files: [] }
  for (const file of files) {
    const parts = file.path.split('/').filter(Boolean)
    let folder = root
    for (const part of parts.slice(0, -1)) {
      if (!folder.folders.has(part)) folder.folders.set(part, { folders: new Map(), files: [] })
      folder = folder.folders.get(part)!
    }
    folder.files.push(file)
  }
  return root
}

export function FileModuleTree({ nodes, onFocusNode }: { nodes: PositionedNode[]; onFocusNode: (id: string) => void }) {
  const [query, setQuery] = useState('')
  const visible = useMemo(() => fileEntries(nodes, query), [nodes, query])
  const tree = useMemo(() => buildTree(visible), [visible])

  return <section className="p-3 text-[11px]"><h2 className="mb-2 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Files & modules</h2><input aria-label="Filter files and modules" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter current graph slice…" className="mb-2 w-full rounded border px-2 py-1" style={{ background: 'var(--atlas-canvas)', borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-bright)' }} />{visible.length ? <Tree folder={tree} onFocusNode={onFocusNode} /> : <p role="status" style={{ color: 'var(--atlas-text-muted)' }}>No files or modules match this filter.</p>}</section>
}

function Tree({ folder, onFocusNode }: { folder: Folder; onFocusNode: (id: string) => void }) {
  const folders = [...folder.folders.entries()].sort(([a], [b]) => a.localeCompare(b))
  const files = [...folder.files].sort((a, b) => a.path.localeCompare(b.path))
  return <ul className="space-y-0.5">{folders.map(([name, child]) => <li key={name}><details open><summary className="cursor-pointer truncate" title={name}>{name}</summary><div className="pl-3"><Tree folder={child} onFocusNode={onFocusNode} /></div></details></li>)}{files.map((file) => <li key={file.path}><button type="button" className="w-full truncate text-left" title={file.path} onClick={() => onFocusNode(file.focusNode.id)}>{file.path} <span style={{ color: 'var(--atlas-text-faint)' }}>· {file.focusNode.label}{file.matches.length > 1 ? ` +${file.matches.length - 1}` : ''}</span></button></li>)}</ul>
}

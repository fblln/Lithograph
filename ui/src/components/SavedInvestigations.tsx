import { useState } from 'react'
import { exportInvestigation, loadInvestigations, saveInvestigation, type SavedInvestigation } from '../investigations'

export function SavedInvestigations({ current, onRestore }: { current: Omit<SavedInvestigation, 'id' | 'name' | 'notes'>; onRestore: (value: SavedInvestigation) => void }) {
  const [name, setName] = useState('Investigation')
  const [notes, setNotes] = useState('')
  const [items, setItems] = useState(loadInvestigations)
  function save() {
    const item: SavedInvestigation = { ...current, id: `${current.graphSnapshotId}:${name}`, name, notes }
    saveInvestigation(item); setItems(loadInvestigations())
  }
  function download(item: SavedInvestigation) {
    const anchor = document.createElement('a')
    anchor.href = `data:application/json;charset=utf-8,${encodeURIComponent(exportInvestigation(item))}`
    anchor.download = `${item.name.trim().replaceAll(/[^a-z0-9]+/gi, '-').replaceAll(/^-|-$/g, '') || 'investigation'}.json`
    anchor.click()
  }
  return <section className="p-3 text-[11px]"><h2>Saved investigations</h2><input aria-label="Investigation name" value={name} onChange={(event) => setName(event.target.value)} /><textarea aria-label="Investigation notes" value={notes} onChange={(event) => setNotes(event.target.value)} /><button type="button" onClick={save}>Save investigation</button><p className="mt-2" style={{ color: 'var(--atlas-text-muted)' }}>Exports are portable, versioned JSON reports.</p><ul>{items.map((item) => <li key={item.id}><button type="button" onClick={() => onRestore(item)}>{item.name}</button>{item.graphSnapshotId !== current.graphSnapshotId && <span role="status"> stale snapshot</span>}<button type="button" aria-label={`Export ${item.name} as JSON`} onClick={() => download(item)}>Export JSON</button></li>)}</ul></section>
}

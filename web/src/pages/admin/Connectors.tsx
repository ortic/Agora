import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Dialog, ErrorText } from '../../ui'

interface AdminConnector {
  id: string
  name: string
  kind: 'wikicraft'
  baseUrl: string
  description: string | null
  enabled: boolean
  connectedUsers: number
}

/** Admin settings → knowledge sources (RAG). */
export function ConnectorsAdmin() {
  const list = useQuery({ queryKey: ['admin-connectors'], queryFn: () => api.get<AdminConnector[]>('/admin/connectors') })
  const [editing, setEditing] = useState<AdminConnector | 'new' | null>(null)
  return (
    <>
      <div className="row">
        <h2 className="grow">Knowledge sources</h2>
        <button className="btn" onClick={() => setEditing('new')}>Add a WikiCraft wiki</button>
      </div>
      <p className="muted" style={{ marginTop: 0 }}>
        Models search these while answering (retrieval-augmented generation). Everyone connects their own read-only token under Settings, so the wiki's
        permissions apply to every search.
      </p>
      {(list.data?.length ?? 0) > 0 && (
        <div className="table-wrap">
          <table className="data">
            <tbody>
              {list.data!.map((c) => (
                <tr key={c.id}>
                  <td><strong>{c.name}</strong> <span className="badge">WikiCraft</span>{!c.enabled && <span className="badge danger" style={{ marginLeft: 4 }}>off</span>}<div className="faint mono">{c.baseUrl}</div></td>
                  <td className="num">{c.connectedUsers} people connected</td>
                  <td style={{ textAlign: 'right' }}><button className="btn small ghost" onClick={() => setEditing(c)}>Edit</button></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {editing && <ConnectorDialog c={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </>
  )
}

function ConnectorDialog({ c, onClose }: { c: AdminConnector | null; onClose: () => void }) {
  const qc = useQueryClient()
  const [f, setF] = useState({ name: c?.name ?? 'Wiki', baseUrl: c?.baseUrl ?? 'http://127.0.0.1:8787', description: c?.description ?? '', enabled: c?.enabled ?? true })
  const done = () => {
    qc.invalidateQueries({ queryKey: ['admin-connectors'] })
    qc.invalidateQueries({ queryKey: ['connectors'] })
    onClose()
  }
  const save = useMutation({ mutationFn: () => (c ? api.patch(`/admin/connectors/${c.id}`, f) : api.post('/admin/connectors', f)), onSuccess: done })
  const remove = useMutation({ mutationFn: () => api.del(`/admin/connectors/${c!.id}`), onSuccess: done })
  return (
    <Dialog title={c ? `Edit ${c.name}` : 'Add a WikiCraft wiki'} onClose={onClose}>
      <label className="field"><span>Name (shown to people and the model)</span><input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} /></label>
      <label className="field"><span>Address</span><input className="input" value={f.baseUrl} onChange={(e) => setF({ ...f, baseUrl: e.target.value })} /></label>
      <label className="field">
        <span>What's in it</span>
        <input className="input" value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} placeholder="e.g. engineering runbooks, HR policies, project documentation" />
        <div className="hint">Helps the model decide when to search it.</div>
      </label>
      <label className="check"><input type="checkbox" checked={f.enabled} onChange={(e) => setF({ ...f, enabled: e.target.checked })} /> Enabled</label>
      <ErrorText error={save.error ?? remove.error} />
      <div className="actions">
        {c && <button className="btn danger" style={{ marginRight: 'auto' }} onClick={() => confirm(`Remove ${c.name}? Everyone's connection to it is removed too.`) && remove.mutate()}>Remove</button>}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!f.name.trim() || save.isPending} onClick={() => save.mutate()}>Save</button>
      </div>
    </Dialog>
  )
}

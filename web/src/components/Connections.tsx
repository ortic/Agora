import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { BookOpen } from 'lucide-react'
import { ago, api, type ConnectorInfo } from '../api'
import { Dialog, ErrorText } from '../ui'

/** Settings → Connections: connect your own account of each knowledge source. */
export function Connections() {
  const qc = useQueryClient()
  const list = useQuery({ queryKey: ['connectors'], queryFn: () => api.get<ConnectorInfo[]>('/connectors') })
  const [connecting, setConnecting] = useState<ConnectorInfo | null>(null)
  const disconnect = useMutation({ mutationFn: (id: string) => api.del(`/connectors/${id}/token`), onSuccess: () => qc.invalidateQueries({ queryKey: ['connectors'] }) })
  if (!list.data?.length) return null
  return (
    <>
      <h2>Knowledge sources</h2>
      <p className="muted" style={{ marginTop: 0 }}>Connect your account so models can search these sources while answering you. They search with your permissions: only what you could open yourself.</p>
      <div className="table-wrap">
        <table className="data">
          <tbody>
            {list.data.map((c) => (
              <tr key={c.id}>
                <td>
                  <BookOpen size={14} style={{ verticalAlign: -2, marginRight: 6 }} />
                  <strong>{c.name}</strong> <span className="faint">{c.baseUrl}</span>
                  {c.description && <div className="faint" style={{ fontSize: 12.5 }}>{c.description}</div>}
                </td>
                <td>{c.connected ? <span className="badge accent">connected {c.tokenHint}</span> : <span className="badge">not connected</span>}</td>
                <td className="faint">{c.connected && `used ${ago(c.lastUsedAt)}`}</td>
                <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                  <button className="btn small" onClick={() => setConnecting(c)}>{c.connected ? 'Replace token' : 'Connect'}</button>{' '}
                  {c.connected && <button className="btn small ghost" onClick={() => disconnect.mutate(c.id)}>Disconnect</button>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {connecting && <ConnectDialog c={connecting} onClose={() => setConnecting(null)} />}
    </>
  )
}

function ConnectDialog({ c, onClose }: { c: ConnectorInfo; onClose: () => void }) {
  const qc = useQueryClient()
  const [token, setToken] = useState('')
  const connect = useMutation({
    mutationFn: () => api.put<{ remoteUser: string }>(`/connectors/${c.id}/token`, { token }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['connectors'] })
      onClose()
    },
  })
  return (
    <Dialog title={`Connect ${c.name}`} onClose={onClose}>
      <ol style={{ paddingLeft: 20, marginTop: 0 }}>
        <li>
          Open <a href={`${c.baseUrl}/settings`} target="_blank" rel="noopener noreferrer">{c.name} → Settings → API tokens</a>.
        </li>
        <li>Create a token with <strong>read-only</strong> access (e.g. named “Agora”).</li>
        <li>Paste it here.</li>
      </ol>
      <label className="field">
        <span>Token</span>
        <input className="input mono" value={token} onChange={(e) => setToken(e.target.value)} placeholder="wct_…" autoFocus autoComplete="off" />
        <div className="hint">Stored encrypted. Agora uses it only to search while answering you; each search is logged.</div>
      </label>
      <ErrorText error={connect.error} />
      <div className="actions">
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!token.trim() || connect.isPending} onClick={() => connect.mutate()}>{connect.isPending ? 'Checking…' : 'Connect'}</button>
      </div>
    </Dialog>
  )
}

import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { KeyRound, Trash2 } from 'lucide-react'
import { ago, api, num, usd, type Budget, type Me } from '../api'
import { applyTheme, Bars, CopyButton, Dialog, ErrorText, getTheme, Meter, type Theme } from '../ui'
import { Connections } from '../components/Connections'

interface MyUsage {
  budget: Budget
  models: { model: string; requests: number; inputTokens: number; outputTokens: number; costUsd: number }[]
  days: { day: string; costUsd: number; requests: number }[]
}

interface Token {
  id: string
  name: string
  createdAt: string
  expiresAt: string | null
  lastUsedAt: string | null
}

export function SettingsPage({ me }: { me: Me }) {
  const usage = useQuery({ queryKey: ['my-usage'], queryFn: () => api.get<MyUsage>('/me/usage') })
  const [theme, setTheme] = useState<Theme>(getTheme())
  const u = usage.data
  const days = lastDays(30, u?.days ?? [])
  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 900 }}>
        <div className="page-head">
          <div>
            <h1>Settings & usage</h1>
            <div className="muted">
              {me.user!.name} · {me.user!.email} · {me.user!.role}
            </div>
          </div>
        </div>

        <h2>This month</h2>
        <div className="stats">
          <div className="stat">
            <div className="label">Spent</div>
            <div className="value">{usd(u?.budget.spent ?? 0, 2)}</div>
            <div className="sub">{u?.budget.limit != null ? `of ${usd(u.budget.limit, 2)} budget (${u.budget.source})` : 'no budget limit'}</div>
            {u && <div style={{ marginTop: 8 }}><Meter value={u.budget.spent} max={u.budget.limit} /></div>}
          </div>
          <div className="stat">
            <div className="label">Requests</div>
            <div className="value">{num(u?.models.reduce((a, m) => a + m.requests, 0) ?? 0)}</div>
          </div>
          <div className="stat">
            <div className="label">Tokens</div>
            <div className="value">{num(u?.models.reduce((a, m) => a + m.inputTokens + m.outputTokens, 0) ?? 0)}</div>
          </div>
        </div>
        <div className="card" style={{ marginBottom: 16 }}>
          <div className="muted" style={{ fontSize: 13 }}>Cost per day, last 30 days</div>
          <Bars values={days.map((d) => d.costUsd)} labels={days.map((d) => d.day)} format={(n) => usd(n)} />
        </div>
        {u && u.models.length > 0 && (
          <div className="table-wrap">
            <table className="data">
              <thead>
                <tr>
                  <th>Model</th>
                  <th className="num">Requests</th>
                  <th className="num">Input tokens</th>
                  <th className="num">Output tokens</th>
                  <th className="num">Cost</th>
                </tr>
              </thead>
              <tbody>
                {u.models.map((m) => (
                  <tr key={m.model}>
                    <td>{m.model}</td>
                    <td className="num">{num(m.requests)}</td>
                    <td className="num">{num(m.inputTokens)}</td>
                    <td className="num">{num(m.outputTokens)}</td>
                    <td className="num">{usd(m.costUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <Connections />

        <Tokens />

        <h2>Appearance</h2>
        <div className="row">
          {(['system', 'light', 'dark'] as Theme[]).map((t) => (
            <label key={t} className="check" style={{ marginBottom: 0 }}>
              <input
                type="radio"
                name="theme"
                checked={theme === t}
                onChange={() => {
                  setTheme(t)
                  applyTheme(t)
                }}
              />
              {t === 'system' ? 'Like the system' : t === 'light' ? 'Light' : 'Dark'}
            </label>
          ))}
        </div>
      </div>
    </div>
  )
}

/** The last `n` days, with zeros for days without usage. */
export function lastDays<T extends { day: string }>(n: number, rows: (T & { costUsd: number })[]): { day: string; costUsd: number }[] {
  const by = new Map(rows.map((r) => [r.day, r.costUsd]))
  const out = []
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(Date.now() - i * 86400000).toISOString().slice(0, 10)
    out.push({ day: d, costUsd: by.get(d) ?? 0 })
  }
  return out
}

function Tokens() {
  const qc = useQueryClient()
  const tokens = useQuery({ queryKey: ['tokens'], queryFn: () => api.get<Token[]>('/me/tokens') })
  const [creating, setCreating] = useState(false)
  const [name, setName] = useState('')
  const [days, setDays] = useState('90')
  const [created, setCreated] = useState<string | null>(null)
  const create = useMutation({
    mutationFn: () => api.post<{ token: string }>('/me/tokens', { name, expiresInDays: days ? Number(days) : null }),
    onSuccess: (r) => {
      setCreated(r.token)
      setCreating(false)
      setName('')
      qc.invalidateQueries({ queryKey: ['tokens'] })
    },
  })
  const remove = useMutation({ mutationFn: (id: string) => api.del(`/me/tokens/${id}`), onSuccess: () => qc.invalidateQueries({ queryKey: ['tokens'] }) })
  const base = `${window.location.origin}/api/v1`
  return (
    <>
      <h2>API tokens</h2>
      <p className="muted" style={{ marginTop: 0 }}>
        Use the models from your own tools and scripts with any OpenAI-compatible client. Base URL <code>{base}</code>. Your budget applies, and every call is
        logged like a chat.
      </p>
      <div className="table-wrap" style={{ marginBottom: 10 }}>
        <table className="data">
          <thead>
            <tr>
              <th>Name</th>
              <th>Created</th>
              <th>Expires</th>
              <th>Last used</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {tokens.data?.map((t) => (
              <tr key={t.id}>
                <td>
                  <KeyRound size={13} style={{ verticalAlign: -2, marginRight: 6 }} />
                  {t.name}
                </td>
                <td>{ago(t.createdAt)}</td>
                <td>{t.expiresAt ? new Date(t.expiresAt).toLocaleDateString() : 'never'}</td>
                <td>{ago(t.lastUsedAt)}</td>
                <td style={{ textAlign: 'right' }}>
                  <button className="icon-btn" aria-label={`Delete ${t.name}`} onClick={() => confirm(`Delete the token “${t.name}”?`) && remove.mutate(t.id)}>
                    <Trash2 size={15} />
                  </button>
                </td>
              </tr>
            ))}
            {tokens.data?.length === 0 && (
              <tr>
                <td colSpan={5} className="empty">No tokens yet.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <button className="btn" onClick={() => setCreating(true)}>New token</button>
      {creating && (
        <Dialog title="New API token" onClose={() => setCreating(false)}>
          <label className="field">
            <span>Name</span>
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. VS Code" autoFocus />
          </label>
          <label className="field">
            <span>Expires after</span>
            <select className="input" value={days} onChange={(e) => setDays(e.target.value)}>
              <option value="30">30 days</option>
              <option value="90">90 days</option>
              <option value="365">1 year</option>
              <option value="">Never</option>
            </select>
          </label>
          <ErrorText error={create.error} />
          <div className="actions">
            <button className="btn" onClick={() => setCreating(false)}>Cancel</button>
            <button className="btn primary" disabled={!name.trim() || create.isPending} onClick={() => create.mutate()}>Create</button>
          </div>
        </Dialog>
      )}
      {created && (
        <Dialog title="Your new token" onClose={() => setCreated(null)}>
          <p>Copy it now: it isn't shown again.</p>
          <div className="row">
            <div className="token-once grow">{created}</div>
            <CopyButton text={created} />
          </div>
          <pre className="json" style={{ marginTop: 14 }}>{`curl ${base}/chat/completions \\
  -H "Authorization: Bearer ${created.slice(0, 8)}…" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "<model>", "messages": [{"role": "user", "content": "Hello"}]}'`}</pre>
          <div className="actions">
            <button className="btn primary" onClick={() => setCreated(null)}>Done</button>
          </div>
        </Dialog>
      )}
    </>
  )
}

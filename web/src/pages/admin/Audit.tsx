import { Fragment, useState } from 'react'
import { Link } from 'react-router-dom'
import { useInfiniteQuery } from '@tanstack/react-query'
import { api, when } from '../../api'

interface AuditEvent {
  id: number
  at: string
  actorId: string | null
  actor: string | null
  ip: string | null
  userAgent: string | null
  action: string
  targetType: string | null
  targetId: string | null
  data: Record<string, unknown>
  hash: string
}

const ACTIONS = [
  ['', 'All events'],
  ['auth.', 'Sign-ins'],
  ['chat.', 'Chats (prompts and answers)'],
  ['web.', 'Web searches and page reads'],
  ['api.', 'API use'],
  ['audit.', 'Audit access (viewed chats, exports)'],
  ['user.', 'People'],
  ['group.', 'Groups'],
  ['provider.', 'Providers'],
  ['model.', 'Models'],
  ['settings.', 'Settings'],
  ['token.', 'API tokens'],
  ['retention.', 'Retention'],
]

export function AuditAdmin() {
  const [action, setAction] = useState('')
  const [q, setQ] = useState('')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [open, setOpen] = useState<number | null>(null)
  const params = new URLSearchParams()
  if (action) params.set('action', action)
  if (q.trim()) params.set('q', q.trim())
  if (from) params.set('from', from)
  if (to) params.set('to', to)
  const events = useInfiniteQuery({
    queryKey: ['audit', params.toString()],
    queryFn: ({ pageParam }) => api.get<{ events: AuditEvent[]; more: boolean }>(`/admin/audit?${params}${pageParam ? `&beforeId=${pageParam}` : ''}`),
    initialPageParam: 0,
    getNextPageParam: (last) => (last.more ? last.events[last.events.length - 1]!.id : undefined),
  })
  const rows = events.data?.pages.flatMap((p) => p.events) ?? []
  return (
    <>
      <div className="row" style={{ marginBottom: 12 }}>
        <select className="input" style={{ width: 'auto' }} value={action} onChange={(e) => setAction(e.target.value)} aria-label="Event type">
          {ACTIONS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
        <input className="input" style={{ maxWidth: 260 }} placeholder="Search details, person, IP" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search" />
        <input type="date" className="input" style={{ width: 150 }} value={from} onChange={(e) => setFrom(e.target.value)} aria-label="From" />
        <input type="date" className="input" style={{ width: 150 }} value={to} onChange={(e) => setTo(e.target.value)} aria-label="To" />
        <div className="grow" />
        <a className="btn" href={`/api/admin/audit/export?${params}&format=csv`}>CSV</a>
        <a className="btn" href={`/api/admin/audit/export?${params}&format=jsonl`}>JSONL</a>
      </div>
      <div className="table-wrap">
        <table className="data">
          <thead>
            <tr><th>#</th><th>Time</th><th>Who</th><th>Event</th><th>Details</th></tr>
          </thead>
          <tbody>
            {rows.map((e) => (
              <Fragment key={e.id}>
                <tr className="clickable" onClick={() => setOpen(open === e.id ? null : e.id)}>
                  <td className="mono faint">{e.id}</td>
                  <td style={{ whiteSpace: 'nowrap' }}>{when(e.at)}</td>
                  <td>{e.actor ?? <span className="faint">system</span>}<div className="faint mono">{e.ip}</div></td>
                  <td><span className="badge">{e.action}</span></td>
                  <td className="muted" style={{ maxWidth: 420, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{summary(e)}</td>
                </tr>
                {open === e.id && (
                  <tr>
                    <td colSpan={5}>
                      {e.targetType === 'chat' && e.targetId && <Link to={`/admin/chats/${e.targetId}`}>Open this conversation (recorded)</Link>}
                      <pre className="json">{JSON.stringify({ target: e.targetType ? `${e.targetType}:${e.targetId}` : null, ...e.data, userAgent: e.userAgent, hash: e.hash }, null, 2)}</pre>
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
            {rows.length === 0 && !events.isLoading && <tr><td colSpan={5} className="empty">No events.</td></tr>}
          </tbody>
        </table>
      </div>
      {events.hasNextPage && (
        <div className="pager"><button className="btn" onClick={() => events.fetchNextPage()} disabled={events.isFetchingNextPage}>Older events</button></div>
      )}
    </>
  )
}

function summary(e: AuditEvent): string {
  const d = e.data as Record<string, any>
  switch (e.action) {
    case 'chat.prompt': return `${d.model}: ${d.content ?? `${d.chars} characters`}`
    case 'chat.answer': return `${d.model} · ${d.status} · ${d.outputTokens} tokens · $${Number(d.costUsd ?? 0).toFixed(4)}${d.content ? ` · ${d.content}` : ''}`
    case 'web.search': return `${d.engine}: “${d.query}” → ${(d.results ?? []).length} results${d.error ? ` (${d.error})` : ''}`
    case 'web.fetch': return `${d.url} (${d.status})`
    case 'api.completion': return `${d.model} · ${d.status}${d.prompt ? ` · ${d.prompt}` : ''}`
    case 'auth.login': return `via ${d.method}${d.provider ? ` (${d.provider})` : ''}`
    case 'auth.login_failed': return `${d.email}`
    default: return JSON.stringify(d)
  }
}

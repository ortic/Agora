import { Fragment, useState } from 'react'
import { Link } from 'react-router-dom'
import { useInfiniteQuery } from '@tanstack/react-query'
import { api, when, type SearchHit } from '../../api'
import { hostOf } from '../../ui'

interface WebEvent {
  id: string
  at: string
  userId: string | null
  user: string | null
  chatId: string | null
  kind: 'search' | 'fetch' | 'retrieve'
  engine: string
  query: string | null
  url: string | null
  results: SearchHit[]
  resultCount: number
  status: 'ok' | 'error' | 'blocked'
  error: string | null
  durationMs: number | null
}

export function WebAdmin() {
  const [kind, setKind] = useState('')
  const [q, setQ] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  const params = new URLSearchParams()
  if (kind) params.set('kind', kind)
  if (q.trim()) params.set('q', q.trim())
  const events = useInfiniteQuery({
    queryKey: ['web-events', params.toString()],
    queryFn: ({ pageParam }) => api.get<{ events: WebEvent[]; more: boolean }>(`/admin/web?${params}${pageParam ? `&before=${encodeURIComponent(pageParam)}` : ''}`),
    initialPageParam: '',
    getNextPageParam: (last) => (last.more ? last.events[last.events.length - 1]!.at : undefined),
  })
  const rows = events.data?.pages.flatMap((p) => p.events) ?? []
  return (
    <>
      <p className="muted" style={{ marginTop: 0 }}>Every web search, page read and knowledge search a model ran, for whom, and what came back.</p>
      <div className="row" style={{ marginBottom: 12 }}>
        <select className="input" style={{ width: 'auto' }} value={kind} onChange={(e) => setKind(e.target.value)} aria-label="Kind">
          <option value="">Everything</option>
          <option value="search">Searches</option>
          <option value="fetch">Page reads</option>
          <option value="retrieve">Knowledge searches (wiki)</option>
        </select>
        <input className="input" style={{ maxWidth: 320 }} placeholder="Search queries, URLs, results" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search" />
      </div>
      <div className="table-wrap">
        <table className="data">
          <thead><tr><th>Time</th><th>Person</th><th>Engine</th><th>Query / URL</th><th className="num">Results</th><th>Status</th></tr></thead>
          <tbody>
            {rows.map((e) => (
              <Fragment key={e.id}>
                <tr className="clickable" onClick={() => setOpen(open === e.id ? null : e.id)}>
                  <td style={{ whiteSpace: 'nowrap' }}>{when(e.at)}</td>
                  <td>{e.user ?? '—'}</td>
                  <td><span className="badge">{e.engine}</span></td>
                  <td style={{ maxWidth: 420, overflowWrap: 'anywhere' }}>{e.kind === 'fetch' ? e.url : `“${e.query}”`}</td>
                  <td className="num">{e.kind === 'fetch' ? '' : e.resultCount}</td>
                  <td>
                    <span className={`badge${e.status === 'ok' ? '' : e.status === 'blocked' ? ' warn' : ' danger'}`}>{e.status}</span>
                    {e.durationMs != null && <span className="faint" style={{ fontSize: 12, marginLeft: 4 }}>{(e.durationMs / 1000).toFixed(1)} s</span>}
                  </td>
                </tr>
                {open === e.id && (
                  <tr>
                    <td colSpan={6}>
                      {e.error && <div className="error-text">{e.error}</div>}
                      {e.results.map((r, i) => (
                        <div key={i} style={{ fontSize: 13 }}>
                          <span className="faint">{hostOf(r.url)}</span> · <a href={r.url} target="_blank" rel="noopener noreferrer">{r.title || r.url}</a>
                        </div>
                      ))}
                      {e.chatId && <div style={{ marginTop: 6 }}><Link to={`/admin/chats/${e.chatId}`}>Open the conversation (recorded)</Link></div>}
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
            {rows.length === 0 && !events.isLoading && <tr><td colSpan={6} className="empty">No web activity yet.</td></tr>}
          </tbody>
        </table>
      </div>
      {events.hasNextPage && <div className="pager"><button className="btn" onClick={() => events.fetchNextPage()}>Older</button></div>}
    </>
  )
}

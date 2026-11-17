import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api, num, usd } from '../../api'

interface OverviewData {
  users: number
  activeLast7Days: number
  chatsToday: number
  costThisMonth: number
  requestsToday: number
  providers: number
  models: number
  webSearch: string
  claudeNativeSearch: boolean
  secretKeySet: boolean
}

interface Verify {
  ok: boolean
  checked: number
  firstBadId: number | null
  head: string | null
  headId: number | null
}

export function Overview() {
  const o = useQuery({ queryKey: ['admin-overview'], queryFn: () => api.get<OverviewData>('/admin/overview') })
  const v = useQuery({ queryKey: ['audit-verify'], queryFn: () => api.get<Verify>('/admin/audit/verify'), staleTime: 60_000 })
  const d = o.data
  return (
    <>
      {d && !d.secretKeySet && (
        <div className="banner" style={{ marginBottom: 16 }}>
          <strong>AGORA_SECRET_KEY isn't set.</strong> Provider API keys are encrypted with a built-in development key. Set a random secret before going live.
        </div>
      )}
      {d && d.models === 0 && (
        <div className="banner ok" style={{ marginBottom: 16 }}>
          Next step: <Link to="/admin/models">connect a provider and add models</Link>.
        </div>
      )}
      <div className="stats">
        <Stat label="People" value={num(d?.users)} sub={`${num(d?.activeLast7Days)} active in the last 7 days`} />
        <Stat label="Cost this month" value={usd(d?.costThisMonth ?? 0, 2)} />
        <Stat label="Model requests today" value={num(d?.requestsToday)} />
        <Stat label="New chats today" value={num(d?.chatsToday)} />
        <Stat label="Models" value={num(d?.models)} sub={`${num(d?.providers)} providers`} />
        <Stat label="Web search" value={d ? (d.webSearch === 'none' ? (d.claudeNativeSearch ? 'Claude' : 'off') : d.webSearch) : '—'} />
      </div>
      <div className="card">
        <h3 style={{ marginTop: 0 }}>Audit log integrity</h3>
        {v.isLoading && <div className="muted">Checking the hash chain…</div>}
        {v.data &&
          (v.data.ok ? (
            <div className="banner ok">
              The hash chain of all {num(v.data.checked)} events is intact. Head #{v.data.headId}: <span className="mono">{v.data.head?.slice(0, 16)}…</span>
            </div>
          ) : (
            <div className="banner error">The audit log was tampered with: event #{v.data.firstBadId} doesn't match its hash or its predecessor.</div>
          ))}
        <div className="hint">Every event's hash covers its content and the previous event's hash; the database refuses changes and deletions. Write down the head hash elsewhere from time to time to detect a rewrite of the whole chain.</div>
      </div>
    </>
  )
}

export function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="stat">
      <div className="label">{label}</div>
      <div className="value">{value}</div>
      {sub && <div className="sub">{sub}</div>}
    </div>
  )
}

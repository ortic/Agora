import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api, num, usd } from '../../api'
import { Bars } from '../../ui'
import { Stat } from './Overview'

interface UsageData {
  from: string
  to: string
  totals: { requests: number; inputTokens: number; outputTokens: number; cacheTokens: number; errors: number; costUsd: number; users: number; searches: number; fetches: number }
  days: { day: string; requests: number; costUsd: number; tokens: number }[]
  users: { userId: string | null; name: string | null; email: string | null; requests: number; inputTokens: number; outputTokens: number; costUsd: number }[]
  models: { model: string; requests: number; inputTokens: number; outputTokens: number; costUsd: number; avgLatencyMs: number | null }[]
  groups: { group: string; requests: number; costUsd: number }[]
}

const iso = (d: Date) => d.toISOString().slice(0, 10)

export function UsageAdmin() {
  const [from, setFrom] = useState(iso(new Date(Date.now() - 29 * 86400000)))
  const [to, setTo] = useState(iso(new Date()))
  const [metric, setMetric] = useState<'costUsd' | 'requests' | 'tokens'>('costUsd')
  const u = useQuery({ queryKey: ['admin-usage', from, to], queryFn: () => api.get<UsageData>(`/admin/usage?from=${from}&to=${to}`), placeholderData: (p) => p })
  const d = u.data
  return (
    <>
      <div className="row" style={{ marginBottom: 14 }}>
        <label className="row" style={{ gap: 6 }}>From <input type="date" className="input" style={{ width: 160 }} value={from} onChange={(e) => setFrom(e.target.value)} /></label>
        <label className="row" style={{ gap: 6 }}>to <input type="date" className="input" style={{ width: 160 }} value={to} onChange={(e) => setTo(e.target.value)} /></label>
        <div className="grow" />
        <a className="btn" href={`/api/admin/usage/export?from=${from}&to=${to}`}>Export CSV</a>
      </div>
      <div className="stats">
        <Stat label="Cost" value={usd(d?.totals.costUsd ?? 0, 2)} />
        <Stat label="Requests" value={num(d?.totals.requests)} sub={`${num(d?.totals.errors)} failed`} />
        <Stat label="Tokens in / out" value={`${num(d?.totals.inputTokens)} / ${num(d?.totals.outputTokens)}`} sub={`${num(d?.totals.cacheTokens)} from cache`} />
        <Stat label="People" value={num(d?.totals.users)} />
        <Stat label="Web searches" value={num(d?.totals.searches)} sub={`${num(d?.totals.fetches)} pages read`} />
      </div>
      <div className="card" style={{ marginBottom: 20 }}>
        <div className="row">
          <div className="muted grow">Per day</div>
          <select className="input" style={{ width: 'auto', height: 30 }} value={metric} onChange={(e) => setMetric(e.target.value as 'costUsd')} aria-label="Metric">
            <option value="costUsd">Cost</option>
            <option value="requests">Requests</option>
            <option value="tokens">Tokens</option>
          </select>
        </div>
        {d && <Bars values={d.days.map((x) => x[metric])} labels={d.days.map((x) => x.day)} format={(n) => (metric === 'costUsd' ? usd(n) : num(n))} />}
      </div>
      <div className="two-col">
        <div>
          <h3>By person</h3>
          <div className="table-wrap">
            <table className="data">
              <thead><tr><th>Person</th><th className="num">Requests</th><th className="num">Tokens</th><th className="num">Cost</th></tr></thead>
              <tbody>
                {d?.users.map((x) => (
                  <tr key={x.userId ?? 'none'}>
                    <td>{x.name ?? 'deleted user'}<div className="faint" style={{ fontSize: 12 }}>{x.email}</div></td>
                    <td className="num">{num(x.requests)}</td>
                    <td className="num">{num(x.inputTokens + x.outputTokens)}</td>
                    <td className="num">{usd(x.costUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
        <div>
          <h3>By model</h3>
          <div className="table-wrap">
            <table className="data">
              <thead><tr><th>Model</th><th className="num">Requests</th><th className="num">Avg. latency</th><th className="num">Cost</th></tr></thead>
              <tbody>
                {d?.models.map((x) => (
                  <tr key={x.model}>
                    <td className="mono">{x.model}</td>
                    <td className="num">{num(x.requests)}</td>
                    <td className="num">{x.avgLatencyMs != null ? `${(x.avgLatencyMs / 1000).toFixed(1)} s` : '—'}</td>
                    <td className="num">{usd(x.costUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {d && d.groups.length > 0 && (
            <>
              <h3>By group</h3>
              <div className="table-wrap">
                <table className="data">
                  <thead><tr><th>Group</th><th className="num">Requests</th><th className="num">Cost</th></tr></thead>
                  <tbody>
                    {d.groups.map((g) => (
                      <tr key={g.group}><td>{g.group}</td><td className="num">{num(g.requests)}</td><td className="num">{usd(g.costUsd)}</td></tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </div>
      </div>
    </>
  )
}

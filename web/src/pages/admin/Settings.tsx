import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type ModelInfo } from '../../api'
import { ErrorText } from '../../ui'
import { ConnectorsAdmin } from './Connectors'

interface OrgSettings {
  name: string
  auditContent: 'full' | 'metadata'
  chatRetentionDays: number | null
  auditRetentionDays: number | null
  defaultMonthlyBudgetUsd: number | null
  rateLimitPerMinute: number | null
  defaultModelId: string | null
  titleModelId: string | null
  systemPrompt: string | null
  notice: string | null
  allowUserSystemPrompt: boolean
  webSearch: {
    engine: 'none' | 'searxng' | 'brave' | 'tavily'
    url: string | null
    apiKeySet: boolean
    apiKey?: string
    maxResults: number
    fetchEnabled: boolean
    claudeNative: boolean
    blockedDomains: string[]
    maxToolCalls: number
  }
  authProviders: { id: string; name: string; type: string }[]
}

export function SettingsAdmin() {
  const qc = useQueryClient()
  const loaded = useQuery({ queryKey: ['admin-settings'], queryFn: () => api.get<OrgSettings>('/admin/settings') })
  const models = useQuery({ queryKey: ['models'], queryFn: () => api.get<ModelInfo[]>('/models') })
  const [s, setS] = useState<OrgSettings | null>(null)
  const [blocked, setBlocked] = useState('')
  const [saved, setSaved] = useState(false)
  useEffect(() => {
    if (loaded.data && !s) {
      setS(loaded.data)
      setBlocked(loaded.data.webSearch.blockedDomains.join('\n'))
    }
  }, [loaded.data, s])
  const save = useMutation({
    mutationFn: () => {
      const { authProviders: _, ...rest } = s!
      return api.put<OrgSettings>('/admin/settings', { ...rest, webSearch: { ...rest.webSearch, blockedDomains: blocked.split(/[\s,]+/).filter(Boolean) } })
    },
    onSuccess: (r) => {
      setS({ ...s!, ...r, authProviders: s!.authProviders, webSearch: { ...r.webSearch, apiKey: undefined } })
      setSaved(true)
      setTimeout(() => setSaved(false), 2500)
      qc.invalidateQueries({ queryKey: ['me'] })
      qc.invalidateQueries({ queryKey: ['models'] })
    },
  })
  if (!s) return null
  const ws = s.webSearch
  const setWs = (p: Partial<OrgSettings['webSearch']>) => setS({ ...s, webSearch: { ...ws, ...p } })
  const numOrNull = (v: string) => (v.trim() === '' ? null : Number(v))
  return (
    <div style={{ maxWidth: 760 }}>
      <h2 style={{ marginTop: 0 }}>General</h2>
      <label className="field">
        <span>Organization name</span>
        <input className="input" value={s.name} onChange={(e) => setS({ ...s, name: e.target.value })} />
      </label>
      <div className="two-col">
        <label className="field">
          <span>Default model for new chats</span>
          <select className="input" value={s.defaultModelId ?? ''} onChange={(e) => setS({ ...s, defaultModelId: e.target.value || null })}>
            <option value="">the first one</option>
            {models.data?.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </select>
        </label>
        <label className="field">
          <span>Model that names chats</span>
          <select className="input" value={s.titleModelId ?? ''} onChange={(e) => setS({ ...s, titleModelId: e.target.value || null })}>
            <option value="">the chat's own model</option>
            {models.data?.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </select>
          <div className="hint">A small, cheap or local model is enough.</div>
        </label>
      </div>
      <label className="field">
        <span>Instructions for every conversation</span>
        <textarea className="input" rows={4} value={s.systemPrompt ?? ''} onChange={(e) => setS({ ...s, systemPrompt: e.target.value || null })} placeholder="e.g. You are the assistant of Acme Corp. Never ask for passwords. Answer in the language of the question." />
      </label>
      <label className="field">
        <span>Notice under the message box</span>
        <input className="input" value={s.notice ?? ''} onChange={(e) => setS({ ...s, notice: e.target.value || null })} placeholder="Conversations are logged. Don't paste customer data into external models." />
      </label>
      <label className="check">
        <input type="checkbox" checked={s.allowUserSystemPrompt} onChange={(e) => setS({ ...s, allowUserSystemPrompt: e.target.checked })} />
        People may add their own instructions per chat
      </label>

      <h2>Budgets & limits</h2>
      <div className="two-col">
        <label className="field">
          <span>Default monthly budget per person (USD)</span>
          <input className="input" inputMode="decimal" value={s.defaultMonthlyBudgetUsd ?? ''} onChange={(e) => setS({ ...s, defaultMonthlyBudgetUsd: numOrNull(e.target.value) })} placeholder="unlimited" />
          <div className="hint">Groups and people can have their own.</div>
        </label>
        <label className="field">
          <span>Requests per person per minute</span>
          <input className="input" inputMode="numeric" value={s.rateLimitPerMinute ?? ''} onChange={(e) => setS({ ...s, rateLimitPerMinute: numOrNull(e.target.value) })} placeholder="unlimited" />
        </label>
      </div>

      <h2>Web search</h2>
      <label className="field">
        <span>Search engine</span>
        <select className="input" value={ws.engine} onChange={(e) => setWs({ engine: e.target.value as OrgSettings['webSearch']['engine'] })}>
          <option value="none">None</option>
          <option value="searxng">SearXNG (self-hosted)</option>
          <option value="brave">Brave Search API</option>
          <option value="tavily">Tavily</option>
        </select>
      </label>
      {ws.engine === 'searxng' && (
        <label className="field">
          <span>SearXNG URL</span>
          <input className="input" value={ws.url ?? ''} onChange={(e) => setWs({ url: e.target.value || null })} placeholder="http://searxng:8080" />
          <div className="hint">Enable the <code>json</code> format in SearXNG's settings.yml.</div>
        </label>
      )}
      {(ws.engine === 'brave' || ws.engine === 'tavily') && (
        <label className="field">
          <span>API key</span>
          <input className="input" type="password" value={ws.apiKey ?? ''} onChange={(e) => setWs({ apiKey: e.target.value })} placeholder={ws.apiKeySet ? 'unchanged (set)' : ''} autoComplete="off" />
        </label>
      )}
      <label className="check">
        <input type="checkbox" checked={ws.claudeNative} onChange={(e) => setWs({ claudeNative: e.target.checked })} />
        <span>Claude models use Anthropic's own web search (billed by Anthropic, $10 per 1,000 searches; queries and result URLs are still logged)</span>
      </label>
      <label className="check">
        <input type="checkbox" checked={ws.fetchEnabled} onChange={(e) => setWs({ fetchEnabled: e.target.checked })} />
        Models may open web pages (private network addresses are always refused)
      </label>
      <div className="two-col">
        <label className="field">
          <span>Results per search</span>
          <input className="input" inputMode="numeric" value={ws.maxResults} onChange={(e) => setWs({ maxResults: Number(e.target.value) || 1 })} />
        </label>
        <label className="field">
          <span>Searches and page reads per answer</span>
          <input className="input" inputMode="numeric" value={ws.maxToolCalls} onChange={(e) => setWs({ maxToolCalls: Number(e.target.value) || 1 })} />
        </label>
      </div>
      <label className="field">
        <span>Blocked domains</span>
        <textarea className="input mono" rows={3} value={blocked} onChange={(e) => setBlocked(e.target.value)} placeholder={'one per line, subdomains included\nexample.com'} />
      </label>

      <ConnectorsAdmin />

      <h2>Audit & retention</h2>
      <label className="field">
        <span>What the audit log keeps of prompts and answers</span>
        <select className="input" value={s.auditContent} onChange={(e) => setS({ ...s, auditContent: e.target.value as 'full' })}>
          <option value="full">The full text</option>
          <option value="metadata">Metadata only (model, size, tokens, cost)</option>
        </select>
        <div className="hint">Check this against your works council / privacy rules. Searches, page reads, sign-ins and admin changes are always logged.</div>
      </label>
      <div className="two-col">
        <label className="field">
          <span>Keep deleted chats for (days)</span>
          <input className="input" inputMode="numeric" value={s.chatRetentionDays ?? ''} onChange={(e) => setS({ ...s, chatRetentionDays: numOrNull(e.target.value) })} placeholder="forever" />
          <div className="hint">Deleted chats stay visible to auditors until then.</div>
        </label>
        <label className="field">
          <span>Keep audit events for (days)</span>
          <input className="input" inputMode="numeric" value={s.auditRetentionDays ?? ''} onChange={(e) => setS({ ...s, auditRetentionDays: numOrNull(e.target.value) })} placeholder="forever" />
        </label>
      </div>

      <h2>Sign-in</h2>
      {s.authProviders.length === 0 ? (
        <p className="muted">Only local passwords. Single sign-on (OIDC: Entra ID, Google, Okta, Keycloak…; GitHub; LDAP / Active Directory; SAML) is configured with <code>AGORA_AUTH_PROVIDERS</code>, see the README.</p>
      ) : (
        <ul>{s.authProviders.map((p) => <li key={p.id}>{p.name} <span className="badge">{p.type}</span></li>)}</ul>
      )}

      <ErrorText error={save.error} />
      <div className="row" style={{ position: 'sticky', bottom: 0, background: 'var(--bg)', padding: '12px 0', borderTop: '1px solid var(--border)', marginTop: 20 }}>
        <button className="btn primary" disabled={save.isPending} onClick={() => save.mutate()}>Save settings</button>
        {saved && <span className="muted">Saved.</span>}
      </div>
    </div>
  )
}

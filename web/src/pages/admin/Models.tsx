import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Plus } from 'lucide-react'
import { api, num, usd } from '../../api'
import { Dialog, ErrorText } from '../../ui'
import type { Group } from './Groups'

interface Provider {
  id: string
  name: string
  kind: 'anthropic' | 'openai' | 'claude_code'
  baseUrl: string
  apiKeyHint: string | null
  headerNames: string[]
  enabled: boolean
  models: number
}

interface AdminModel {
  id: string
  providerId: string
  model: string
  displayName: string
  description: string | null
  enabled: boolean
  inputPrice: number
  outputPrice: number
  cacheReadPrice: number
  cacheWritePrice: number
  searchPrice: number
  contextWindow: number | null
  maxOutputTokens: number | null
  capabilities: { vision?: boolean; tools?: boolean; reasoning?: boolean }
  params: { effort?: string; systemPrompt?: string; temperature?: number }
  access: 'all' | 'groups'
  position: number
  providerKind: string
  providerName: string
  providerEnabled: boolean
  groupIds: string[]
}

interface Discovered {
  model: string
  displayName: string
  contextWindow: number | null
  maxOutputTokens: number | null
  added: boolean
}

const PRESETS = [
  { label: 'Anthropic (Claude)', kind: 'anthropic', name: 'Anthropic', baseUrl: '' },
  { label: 'Claude Code on this server (claude -p)', kind: 'claude_code', name: 'Claude Code', baseUrl: '' },
  { label: 'OpenAI', kind: 'openai', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1' },
  { label: 'Ollama on this machine', kind: 'openai', name: 'Ollama (local)', baseUrl: 'http://127.0.0.1:11434/v1' },
  { label: 'vLLM / llama.cpp / LM Studio', kind: 'openai', name: 'Local server', baseUrl: 'http://127.0.0.1:8000/v1' },
  { label: 'Other OpenAI-compatible (LiteLLM, OpenRouter, Azure gateway…)', kind: 'openai', name: '', baseUrl: '' },
] as const

export function ModelsAdmin() {
  const providers = useQuery({ queryKey: ['admin-providers'], queryFn: () => api.get<Provider[]>('/admin/providers') })
  const models = useQuery({ queryKey: ['admin-models'], queryFn: () => api.get<AdminModel[]>('/admin/models') })
  const [editProvider, setEditProvider] = useState<Provider | 'new' | null>(null)
  const [discover, setDiscover] = useState<Provider | null>(null)
  const [editModel, setEditModel] = useState<AdminModel | { providerId: string } | null>(null)
  return (
    <>
      <div className="row">
        <h2 className="grow" style={{ marginTop: 0 }}>Providers</h2>
        <button className="btn primary" onClick={() => setEditProvider('new')}>
          <Plus size={15} /> Connect a provider
        </button>
      </div>
      <div className="table-wrap" style={{ marginBottom: 24 }}>
        <table className="data">
          <thead>
            <tr>
              <th>Provider</th>
              <th>Base URL</th>
              <th>Key</th>
              <th className="num">Models</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {providers.data?.map((p) => (
              <tr key={p.id}>
                <td>
                  <div style={{ fontWeight: 600 }}>{p.name}</div>
                  <span className="badge">{p.kind === 'anthropic' ? 'Anthropic' : p.kind === 'claude_code' ? 'Claude Code' : 'OpenAI-compatible'}</span>
                  {!p.enabled && <span className="badge danger" style={{ marginLeft: 4 }}>off</span>}
                </td>
                <td className="mono">{p.baseUrl}</td>
                <td className="mono">{p.apiKeyHint ?? '—'}</td>
                <td className="num">{p.models}</td>
                <td style={{ whiteSpace: 'nowrap', textAlign: 'right' }}>
                  <button className="btn small" onClick={() => setDiscover(p)}>Add models</button>{' '}
                  <button className="btn small ghost" onClick={() => setEditProvider(p)}>Edit</button>
                </td>
              </tr>
            ))}
            {providers.data?.length === 0 && (
              <tr>
                <td colSpan={5} className="empty">No providers yet. Connect Claude, OpenAI, or a model running in your own network.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <h2>Models</h2>
      <div className="table-wrap">
        <table className="data">
          <thead>
            <tr>
              <th>Model</th>
              <th>Provider</th>
              <th className="num">Input / output per M tokens</th>
              <th className="num">Context</th>
              <th>Access</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {models.data?.map((m) => (
              <tr key={m.id} style={m.enabled ? undefined : { opacity: 0.55 }}>
                <td>
                  <div style={{ fontWeight: 600 }}>{m.displayName}</div>
                  <div className="mono faint">{m.model}</div>
                </td>
                <td>{m.providerName}</td>
                <td className="num">
                  {usd(m.inputPrice, 2)} / {usd(m.outputPrice, 2)}
                </td>
                <td className="num">{m.contextWindow ? num(m.contextWindow) : '—'}</td>
                <td>
                  {!m.enabled ? <span className="badge danger">off</span> : m.access === 'all' ? <span className="badge accent">everyone</span> : <span className="badge warn">{m.groupIds.length} groups</span>}
                </td>
                <td style={{ textAlign: 'right' }}>
                  <button className="btn small ghost" onClick={() => setEditModel(m)}>Edit</button>
                </td>
              </tr>
            ))}
            {models.data?.length === 0 && (
              <tr>
                <td colSpan={6} className="empty">No models yet: use “Add models” on a provider.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {editProvider && <ProviderDialog provider={editProvider === 'new' ? null : editProvider} onClose={() => setEditProvider(null)} />}
      {discover && <DiscoverDialog provider={discover} onClose={() => setDiscover(null)} onManual={() => { setEditModel({ providerId: discover.id }); setDiscover(null) }} />}
      {editModel && <ModelDialog model={'id' in editModel ? editModel : null} providerId={editModel.providerId} onClose={() => setEditModel(null)} />}
    </>
  )
}

function ProviderDialog({ provider, onClose }: { provider: Provider | null; onClose: () => void }) {
  const qc = useQueryClient()
  const [preset, setPreset] = useState(0)
  const [form, setForm] = useState({
    name: provider?.name ?? PRESETS[0].name,
    kind: provider?.kind ?? PRESETS[0].kind,
    baseUrl: provider?.baseUrl ?? '',
    apiKey: '',
    headers: '',
    enabled: provider?.enabled ?? true,
  })
  const save = useMutation({
    mutationFn: async () => {
      let headers: unknown = undefined
      if (form.headers.trim()) {
        try {
          headers = JSON.parse(form.headers)
        } catch {
          throw new Error('Extra headers must be a JSON object, e.g. {"OpenAI-Organization": "org-…"}')
        }
      }
      const body = { name: form.name, kind: form.kind, baseUrl: form.baseUrl || undefined, apiKey: form.apiKey || undefined, headers, enabled: form.enabled }
      return provider ? api.patch(`/admin/providers/${provider.id}`, body) : api.post('/admin/providers', body)
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-providers'] })
      qc.invalidateQueries({ queryKey: ['admin-models'] })
      onClose()
    },
  })
  const remove = useMutation({
    mutationFn: () => api.del(`/admin/providers/${provider!.id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-providers'] })
      qc.invalidateQueries({ queryKey: ['admin-models'] })
      onClose()
    },
  })
  return (
    <Dialog title={provider ? `Edit ${provider.name}` : 'Connect a provider'} onClose={onClose}>
      {!provider && (
        <label className="field">
          <span>Type</span>
          <select
            className="input"
            value={preset}
            onChange={(e) => {
              const p = PRESETS[Number(e.target.value)]!
              setPreset(Number(e.target.value))
              setForm({ ...form, kind: p.kind, name: p.name, baseUrl: p.baseUrl })
            }}
          >
            {PRESETS.map((p, i) => (
              <option key={i} value={i}>{p.label}</option>
            ))}
          </select>
        </label>
      )}
      <label className="field">
        <span>Name</span>
        <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
      </label>
      <label className="field">
        <span>Base URL</span>
        <input className="input" value={form.baseUrl} onChange={(e) => setForm({ ...form, baseUrl: e.target.value })} placeholder={form.kind === 'anthropic' ? 'https://api.anthropic.com' : 'https://…/v1'} />
        {form.kind === 'claude_code' && (
          <div className="hint">
            Answers are written by Claude Code running on the server (it must be installed there). It gets Agora's tools only: web search, your knowledge sources and people's desktop tools; none of its own tools for files, commands or the web. Leave the URL empty.
          </div>
        )}
        {form.kind === 'openai' && <div className="hint">Up to and including <code>/v1</code>; Agora appends <code>/chat/completions</code> and <code>/models</code>.</div>}
      </label>
      <label className="field">
        <span>API key</span>
        <input className="input" type="password" value={form.apiKey} onChange={(e) => setForm({ ...form, apiKey: e.target.value })} placeholder={provider?.apiKeyHint ? `unchanged (${provider.apiKeyHint})` : form.kind === 'anthropic' ? 'sk-ant-…' : form.kind === 'claude_code' ? 'optional: without one, the account claude is signed in with on the server' : 'optional for local servers'} autoComplete="off" />
        <div className="hint">Stored encrypted; never shown again.</div>
      </label>
      <label className="field">
        <span>Extra headers (JSON)</span>
        <input className="input mono" value={form.headers} onChange={(e) => setForm({ ...form, headers: e.target.value })} placeholder={provider?.headerNames.length ? `unchanged (${provider.headerNames.join(', ')})` : '{"X-Header": "value"}'} />
      </label>
      <label className="check">
        <input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /> Enabled
      </label>
      <ErrorText error={save.error ?? remove.error} />
      <div className="actions">
        {provider && (
          <button className="btn danger" style={{ marginRight: 'auto' }} onClick={() => confirm(`Remove ${provider.name} and its ${provider.models} models?`) && remove.mutate()}>
            Remove
          </button>
        )}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={save.isPending} onClick={() => save.mutate()}>{provider ? 'Save' : 'Connect'}</button>
      </div>
    </Dialog>
  )
}

function DiscoverDialog({ provider, onClose, onManual }: { provider: Provider; onClose: () => void; onManual: () => void }) {
  const qc = useQueryClient()
  const found = useQuery({ queryKey: ['discover', provider.id], queryFn: () => api.get<Discovered[]>(`/admin/providers/${provider.id}/discover`), retry: false })
  const [added, setAdded] = useState<Set<string>>(new Set())
  const add = useMutation({
    mutationFn: (d: Discovered) =>
      api.post('/admin/models', { providerId: provider.id, model: d.model, displayName: d.displayName !== d.model ? d.displayName : undefined, contextWindow: d.contextWindow, maxOutputTokens: null }),
    onSuccess: (_, d) => {
      setAdded(new Set([...added, d.model]))
      qc.invalidateQueries({ queryKey: ['admin-models'] })
      qc.invalidateQueries({ queryKey: ['admin-providers'] })
      qc.invalidateQueries({ queryKey: ['models'] })
    },
  })
  return (
    <Dialog title={`Models of ${provider.name}`} onClose={onClose} wide>
      {found.isLoading && <div className="row muted"><span className="spinner" /> Asking the provider…</div>}
      {found.isError && <ErrorText error={found.error} />}
      {found.data && (
        <div className="table-wrap" style={{ maxHeight: 420, overflowY: 'auto' }}>
          <table className="data">
            <tbody>
              {found.data.map((d) => (
                <tr key={d.model}>
                  <td>
                    <div style={{ fontWeight: 600 }}>{d.displayName}</div>
                    <div className="mono faint">{d.model}</div>
                  </td>
                  <td className="num">{d.contextWindow ? `${num(d.contextWindow)} context` : ''}</td>
                  <td style={{ textAlign: 'right' }}>
                    {d.added || added.has(d.model) ? (
                      <span className="badge accent">added</span>
                    ) : (
                      <button className="btn small" disabled={add.isPending} onClick={() => add.mutate(d)}>Add</button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <ErrorText error={add.error} />
      <div className="actions">
        <button className="btn" style={{ marginRight: 'auto' }} onClick={onManual}>Enter a model by hand</button>
        <button className="btn primary" onClick={onClose}>Done</button>
      </div>
      <p className="hint">Claude models get their list prices automatically. For other providers, enter prices under Edit so usage costs are right.</p>
    </Dialog>
  )
}

function ModelDialog({ model, providerId, onClose }: { model: AdminModel | null; providerId: string; onClose: () => void }) {
  const qc = useQueryClient()
  const groups = useQuery({ queryKey: ['admin-groups'], queryFn: () => api.get<Group[]>('/admin/groups') })
  const [f, setF] = useState({
    model: model?.model ?? '',
    displayName: model?.displayName ?? '',
    description: model?.description ?? '',
    enabled: model?.enabled ?? true,
    inputPrice: String(model?.inputPrice ?? ''),
    outputPrice: String(model?.outputPrice ?? ''),
    cacheReadPrice: String(model?.cacheReadPrice ?? ''),
    cacheWritePrice: String(model?.cacheWritePrice ?? ''),
    contextWindow: model?.contextWindow != null ? String(model.contextWindow) : '',
    maxOutputTokens: model?.maxOutputTokens != null ? String(model.maxOutputTokens) : '',
    vision: model?.capabilities.vision ?? false,
    tools: model?.capabilities.tools ?? true,
    reasoning: model?.capabilities.reasoning ?? false,
    effort: model?.params.effort ?? '',
    systemPrompt: model?.params.systemPrompt ?? '',
    temperature: model?.params.temperature != null ? String(model.params.temperature) : '',
    access: model?.access ?? 'all',
    groupIds: new Set(model?.groupIds ?? []),
    position: String(model?.position ?? 0),
  })
  const isClaude = (model?.providerKind ?? '') === 'anthropic'
  const n = (s: string) => (s.trim() === '' ? null : Number(s))
  const save = useMutation({
    mutationFn: () => {
      const params: Record<string, unknown> = { ...(model?.params ?? {}) }
      params.effort = f.effort || undefined
      params.systemPrompt = f.systemPrompt || undefined
      params.temperature = n(f.temperature) ?? undefined
      const body = {
        providerId,
        model: f.model,
        displayName: f.displayName,
        description: f.description,
        enabled: f.enabled,
        inputPrice: n(f.inputPrice) ?? undefined,
        outputPrice: n(f.outputPrice) ?? undefined,
        cacheReadPrice: n(f.cacheReadPrice) ?? undefined,
        cacheWritePrice: n(f.cacheWritePrice) ?? undefined,
        contextWindow: n(f.contextWindow),
        maxOutputTokens: n(f.maxOutputTokens),
        capabilities: { vision: f.vision, tools: f.tools, reasoning: f.reasoning },
        params,
        access: f.access,
        groupIds: [...f.groupIds],
        position: Number(f.position) || 0,
      }
      return model ? api.patch(`/admin/models/${model.id}`, body) : api.post('/admin/models', body)
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-models'] })
      qc.invalidateQueries({ queryKey: ['admin-providers'] })
      qc.invalidateQueries({ queryKey: ['models'] })
      onClose()
    },
  })
  const remove = useMutation({
    mutationFn: () => api.del(`/admin/models/${model!.id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-models'] })
      qc.invalidateQueries({ queryKey: ['models'] })
      onClose()
    },
  })
  const text = (k: keyof typeof f) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) => setF({ ...f, [k]: e.target.value })
  return (
    <Dialog title={model ? `Edit ${model.displayName}` : 'Add a model'} onClose={onClose} wide>
      <div className="two-col">
        <label className="field">
          <span>Model name at the provider</span>
          <input className="input mono" value={f.model} onChange={text('model')} placeholder="llama3.3:70b" />
        </label>
        <label className="field">
          <span>Display name</span>
          <input className="input" value={f.displayName} onChange={text('displayName')} />
        </label>
      </div>
      <label className="field">
        <span>Description (shown in the model picker)</span>
        <input className="input" value={f.description} onChange={text('description')} placeholder="e.g. Fast, runs in our data centre: fine for confidential data" />
      </label>
      <h3>Prices (USD per million tokens)</h3>
      <div className="two-col">
        <label className="field"><span>Input</span><input className="input" inputMode="decimal" value={f.inputPrice} onChange={text('inputPrice')} /></label>
        <label className="field"><span>Output</span><input className="input" inputMode="decimal" value={f.outputPrice} onChange={text('outputPrice')} /></label>
        <label className="field"><span>Cache read</span><input className="input" inputMode="decimal" value={f.cacheReadPrice} onChange={text('cacheReadPrice')} /></label>
        <label className="field"><span>Cache write</span><input className="input" inputMode="decimal" value={f.cacheWritePrice} onChange={text('cacheWritePrice')} /></label>
      </div>
      <div className="hint" style={{ marginTop: -8, marginBottom: 12 }}>Local models: enter 0, or an internal rate to make costs comparable.</div>
      <h3>Limits & behaviour</h3>
      <div className="two-col">
        <label className="field"><span>Context window (tokens)</span><input className="input" inputMode="numeric" value={f.contextWindow} onChange={text('contextWindow')} /></label>
        <label className="field"><span>Max output tokens</span><input className="input" inputMode="numeric" value={f.maxOutputTokens} onChange={text('maxOutputTokens')} placeholder={isClaude ? '32000' : 'server default'} /></label>
        <label className="field">
          <span>Default effort</span>
          <select className="input" value={f.effort} onChange={text('effort')}>
            <option value="">model default</option>
            {['low', 'medium', 'high', 'xhigh', 'max'].map((e) => <option key={e}>{e}</option>)}
          </select>
        </label>
        {!isClaude && (
          <label className="field"><span>Temperature</span><input className="input" inputMode="decimal" value={f.temperature} onChange={text('temperature')} placeholder="server default" /></label>
        )}
      </div>
      <label className="field">
        <span>Model instructions (system prompt)</span>
        <textarea className="input" rows={3} value={f.systemPrompt} onChange={text('systemPrompt')} />
      </label>
      <div className="row" style={{ gap: 18 }}>
        <label className="check"><input type="checkbox" checked={f.tools} onChange={(e) => setF({ ...f, tools: e.target.checked })} /> Tools (web search)</label>
        <label className="check"><input type="checkbox" checked={f.vision} onChange={(e) => setF({ ...f, vision: e.target.checked })} /> Images</label>
        <label className="check"><input type="checkbox" checked={f.reasoning} onChange={(e) => setF({ ...f, reasoning: e.target.checked })} /> Shows its thinking (Claude)</label>
        <label className="check"><input type="checkbox" checked={f.enabled} onChange={(e) => setF({ ...f, enabled: e.target.checked })} /> Enabled</label>
      </div>
      <h3>Who may use it</h3>
      <div className="row" style={{ gap: 18 }}>
        <label className="check"><input type="radio" checked={f.access === 'all'} onChange={() => setF({ ...f, access: 'all' })} /> Everyone</label>
        <label className="check"><input type="radio" checked={f.access === 'groups'} onChange={() => setF({ ...f, access: 'groups' })} /> Only these groups</label>
      </div>
      {f.access === 'groups' && (
        <div className="row" style={{ gap: 14 }}>
          {groups.data?.map((g) => (
            <label key={g.id} className="check">
              <input
                type="checkbox"
                checked={f.groupIds.has(g.id)}
                onChange={(e) => {
                  const s = new Set(f.groupIds)
                  if (e.target.checked) s.add(g.id)
                  else s.delete(g.id)
                  setF({ ...f, groupIds: s })
                }}
              />
              {g.name}
            </label>
          ))}
          {groups.data?.length === 0 && <span className="faint">No groups yet: create them under Groups.</span>}
        </div>
      )}
      <ErrorText error={save.error ?? remove.error} />
      <div className="actions">
        {model && <button className="btn danger" style={{ marginRight: 'auto' }} onClick={() => confirm(`Remove ${model.displayName}?`) && remove.mutate()}>Remove</button>}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!f.model.trim() || save.isPending} onClick={() => save.mutate()}>Save</button>
      </div>
    </Dialog>
  )
}

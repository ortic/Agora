import { Fragment, useState } from 'react'
import { Link } from 'react-router-dom'
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ago, api, when, type Device } from '../../api'
import { Dialog, ErrorText } from '../../ui'

type Access = 'nobody' | 'all' | 'groups'
type FeatureKey = 'desktop' | 'code' | 'browser'

interface Feature {
  access: Access
  config: Record<string, unknown>
  groupIds: string[]
}

interface LocalConnector {
  id: string
  name: string
  kind: 'mcp_stdio' | 'mcp_http'
  description: string | null
  config: { command?: string; args?: string[]; env?: Record<string, string>; url?: string; headers?: Record<string, string> }
  tools: string[]
  approval: 'ask' | 'auto'
  enabled: boolean
  access: 'all' | 'groups'
  groupIds: string[]
}

interface DesktopAdminData {
  features: Record<FeatureKey, Feature>
  connectors: LocalConnector[]
  devices: Device[]
}

interface Group {
  id: string
  name: string
}

const lines = (s: string) => s.split('\n').map((l) => l.trim()).filter(Boolean)
const useGroups = () => useQuery({ queryKey: ['admin-groups'], queryFn: () => api.get<Group[]>('/admin/groups') })

/** Admin → Desktop: what Agora Desktop may do on people's computers, and for whom. */
export function DesktopAdmin({ admin }: { admin: boolean }) {
  const data = useQuery({ queryKey: ['admin-desktop'], queryFn: () => api.get<DesktopAdminData>('/admin/desktop'), enabled: admin })
  const groups = useGroups()
  const [editing, setEditing] = useState<LocalConnector | 'new' | null>(null)
  const d = data.data
  return (
    <>
      <p className="muted" style={{ marginTop: 0 }}>
        Agora Desktop is the app on people's computers. Models keep running here; through the app they can work on the computer: code in dev containers, a
        browser, local connectors. Nothing is available until you grant it, and every tool call is recorded.
      </p>
      {admin && <ErrorText error={data.error} />}
      {admin && d && (
        <>
          <h2>Features</h2>
          <FeatureCard
            k="desktop"
            title="Agora Desktop"
            about="Who may sign in with the app at all. Without this, nothing below applies to a person."
            feature={d.features.desktop}
            groups={groups.data ?? []}
          />
          <FeatureCard
            k="code"
            title="Code in dev containers"
            about="Models read and change a project's files and run commands, inside a container that only has the project folder. Needs Agora Desktop itself granted too."
            feature={d.features.code}
            groups={groups.data ?? []}
          />
          <FeatureCard
            k="browser"
            title="Browser automation"
            about="Models open pages, click and type in a browser window of the app (a profile of its own, not the person's browser). Needs Agora Desktop itself granted too."
            feature={d.features.browser}
            groups={groups.data ?? []}
          />

          <div className="row" style={{ marginTop: 24 }}>
            <h2 className="grow">Local connectors</h2>
            <button className="btn" onClick={() => setEditing('new')}>Add a connector</button>
          </div>
          <p className="muted" style={{ marginTop: 0 }}>
            MCP servers the app starts, or reaches, on the person's computer: local applications, files, internal services only reachable from there. Only what
            is listed here can be used; people can't add their own.
          </p>
          {d.connectors.length > 0 && (
            <div className="table-wrap">
              <table className="data">
                <tbody>
                  {d.connectors.map((c) => (
                    <tr key={c.id}>
                      <td>
                        <strong>{c.name}</strong> <span className="badge">{c.kind === 'mcp_stdio' ? 'program' : 'URL'}</span>
                        {!c.enabled && <span className="badge danger" style={{ marginLeft: 4 }}>off</span>}
                        <div className="faint mono" style={{ overflowWrap: 'anywhere' }}>{c.kind === 'mcp_stdio' ? [c.config.command, ...(c.config.args ?? [])].join(' ') : c.config.url}</div>
                      </td>
                      <td>
                        {c.access === 'all' ? <span className="badge accent">everyone</span> : <span className="badge warn">{c.groupIds.length} groups</span>}{' '}
                        <span className="badge">{c.approval === 'ask' ? 'asks first' : 'runs without asking'}</span>
                      </td>
                      <td className="faint">{c.tools.length ? `${c.tools.length} tools allowed` : 'all its tools'}</td>
                      <td style={{ textAlign: 'right' }}><button className="btn small ghost" onClick={() => setEditing(c)}>Edit</button></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          <h2 style={{ marginTop: 24 }}>Devices</h2>
          <Devices devices={d.devices} />
        </>
      )}

      <h2 style={{ marginTop: admin ? 24 : 0 }}>Tool activity</h2>
      <ToolActivity />
      {editing && <ConnectorDialog c={editing === 'new' ? null : editing} groups={groups.data ?? []} onClose={() => setEditing(null)} />}
    </>
  )
}

function AccessPicker<A extends string>(props: {
  name: string
  access: A
  options: [A, string][]
  groupIds: Set<string>
  groups: Group[]
  onChange: (access: A, groupIds: Set<string>) => void
}) {
  return (
    <>
      <div className="row" style={{ gap: 18 }}>
        {props.options.map(([value, label]) => (
          <label key={value} className="check">
            <input type="radio" name={props.name} checked={props.access === value} onChange={() => props.onChange(value, props.groupIds)} /> {label}
          </label>
        ))}
      </div>
      {props.access === 'groups' && (
        <div className="row" style={{ gap: 14 }}>
          {props.groups.map((g) => (
            <label key={g.id} className="check">
              <input
                type="checkbox"
                checked={props.groupIds.has(g.id)}
                onChange={(e) => {
                  const s = new Set(props.groupIds)
                  if (e.target.checked) s.add(g.id)
                  else s.delete(g.id)
                  props.onChange(props.access, s)
                }}
              />
              {g.name}
            </label>
          ))}
          {props.groups.length === 0 && <span className="faint">No groups yet: create them under Groups.</span>}
        </div>
      )}
    </>
  )
}

const list = (v: unknown) => (Array.isArray(v) ? v.join('\n') : '')
const text = (v: unknown, fallback = '') => (v == null ? fallback : String(v))

function FeatureCard({ k, title, about, feature, groups }: { k: FeatureKey; title: string; about: string; feature: Feature; groups: Group[] }) {
  const qc = useQueryClient()
  const c = feature.config
  const [f, setF] = useState({
    access: feature.access,
    groupIds: new Set(feature.groupIds),
    maxToolCalls: text(c.maxToolCalls, '60'),
    toolTimeoutSecs: text(c.toolTimeoutSecs, '600'),
    subagents: c.subagents !== false,
    maxSubagentToolCalls: text(c.maxSubagentToolCalls, '30'),
    commands: (Array.isArray(c.commands) ? c.commands : []) as { name: string; description: string; prompt: string }[],
    fileCommands: c.fileCommands !== false,
    images: list(c.images),
    defaultImage: text(c.defaultImage),
    network: c.network !== false,
    approval: text(c.approval, 'ask'),
    commandTimeoutSecs: text(c.commandTimeoutSecs, '300'),
    allowedDomains: list(c.allowedDomains),
    blockedDomains: list(c.blockedDomains),
  })
  const [dirty, setDirty] = useState(false)
  const set = (patch: Partial<typeof f>) => {
    setF({ ...f, ...patch })
    setDirty(true)
  }
  const int = (s: string, fallback: number) => {
    const n = Math.round(Number(s))
    return Number.isFinite(n) && n > 0 ? n : fallback
  }
  const save = useMutation({
    mutationFn: () => {
      const config =
        k === 'desktop'
          ? {
              maxToolCalls: int(f.maxToolCalls, 60),
              toolTimeoutSecs: int(f.toolTimeoutSecs, 600),
              subagents: f.subagents,
              maxSubagentToolCalls: int(f.maxSubagentToolCalls, 30),
              commands: f.commands.filter((x) => x.name.trim() || x.prompt.trim()),
              fileCommands: f.fileCommands,
            }
          : k === 'code'
            ? { images: lines(f.images), defaultImage: f.defaultImage.trim(), network: f.network, approval: f.approval, commandTimeoutSecs: int(f.commandTimeoutSecs, 300) }
            : { allowedDomains: lines(f.allowedDomains), blockedDomains: lines(f.blockedDomains), approval: f.approval }
      return api.put(`/admin/desktop/features/${k}`, { access: f.access, config, groupIds: [...f.groupIds] })
    },
    onSuccess: () => {
      setDirty(false)
      qc.invalidateQueries({ queryKey: ['admin-desktop'] })
    },
  })
  const approval = (what: string) => (
    <label className="field">
      <span>Before {what}</span>
      <select className="input" value={f.approval} onChange={(e) => set({ approval: e.target.value })}>
        <option value="ask">The person confirms each one in the app</option>
        <option value="auto">Run without asking</option>
      </select>
    </label>
  )
  return (
    <div className="card feature-card">
      <div className="row">
        <h3 className="grow">{title}</h3>
        {f.access === 'nobody' ? <span className="badge">off</span> : f.access === 'all' ? <span className="badge accent">everyone</span> : <span className="badge warn">{f.groupIds.size} groups</span>}
      </div>
      <p className="muted" style={{ marginTop: 0 }}>{about}</p>
      <AccessPicker
        name={`access-${k}`}
        access={f.access}
        options={[['nobody', 'Nobody'], ['all', 'Everyone'], ['groups', 'Only these groups']]}
        groupIds={f.groupIds}
        groups={groups}
        onChange={(access, groupIds) => set({ access, groupIds })}
      />
      {f.access !== 'nobody' && k === 'desktop' && (
        <div className="two-col">
          <label className="field">
            <span>Tool calls per answer</span>
            <input className="input" inputMode="numeric" value={f.maxToolCalls} onChange={(e) => set({ maxToolCalls: e.target.value })} />
            <div className="hint">Work on code takes many steps; web search alone has its own, lower limit.</div>
          </label>
          <label className="field">
            <span>Seconds a tool may take</span>
            <input className="input" inputMode="numeric" value={f.toolTimeoutSecs} onChange={(e) => set({ toolTimeoutSecs: e.target.value })} />
            <div className="hint">Includes the time the person takes to confirm.</div>
          </label>
          <label className="field">
            <span>Sub-agents</span>
            <select className="input" value={f.subagents ? 'on' : 'off'} onChange={(e) => set({ subagents: e.target.value === 'on' })}>
              <option value="on">Models may hand parts of the work to sub-agents</option>
              <option value="off">Off</option>
            </select>
            <div className="hint">A sub-agent is the same model with the same tools and an empty context. Its model calls are billed like any other.</div>
          </label>
          {f.subagents && (
            <label className="field">
              <span>Tool calls per sub-agent</span>
              <input className="input" inputMode="numeric" value={f.maxSubagentToolCalls} onChange={(e) => set({ maxSubagentToolCalls: e.target.value })} />
              <div className="hint">On top of the answer's own limit.</div>
            </label>
          )}
        </div>
      )}
      {f.access !== 'nobody' && k === 'desktop' && (
        <div style={{ marginTop: 12 }}>
          <div className="row">
            <strong className="grow">Slash commands</strong>
            <button className="btn" onClick={() => set({ commands: [...f.commands, { name: '', description: '', prompt: '' }] })}>
              Add a command
            </button>
          </div>
          <div className="hint">
            Typing /name in the app sends the prompt. <code>$ARGUMENTS</code> in the prompt stands for what the person types after the name.
          </div>
          {f.commands.map((cmd, i) => {
            const change = (patch: Partial<typeof cmd>) => set({ commands: f.commands.map((x, j) => (j === i ? { ...x, ...patch } : x)) })
            return (
              <div key={i} className="card" style={{ marginTop: 8 }}>
                <div className="two-col">
                  <label className="field">
                    <span>Name</span>
                    <input className="input" value={cmd.name} placeholder="review" onChange={(e) => change({ name: e.target.value })} />
                  </label>
                  <label className="field">
                    <span>What it does (shown in the list)</span>
                    <input className="input" value={cmd.description} onChange={(e) => change({ description: e.target.value })} />
                  </label>
                </div>
                <label className="field">
                  <span>Prompt</span>
                  <textarea className="input" rows={4} value={cmd.prompt} onChange={(e) => change({ prompt: e.target.value })} />
                </label>
                <button className="btn" onClick={() => set({ commands: f.commands.filter((_, j) => j !== i) })}>
                  Remove
                </button>
              </div>
            )
          })}
          <label className="field" style={{ marginTop: 10 }}>
            <span>Commands from files</span>
            <select className="input" value={f.fileCommands ? 'on' : 'off'} onChange={(e) => set({ fileCommands: e.target.value === 'on' })}>
              <option value="on">People may add their own (in a project's .agora/commands, or in the app's settings folder)</option>
              <option value="off">Only the commands above</option>
            </select>
          </label>
        </div>
      )}
      {f.access !== 'nobody' && k === 'code' && (
        <>
          <div className="two-col">
            <label className="field">
              <span>Allowed container images (one per line)</span>
              <textarea className="input mono" rows={3} value={f.images} onChange={(e) => set({ images: e.target.value })} placeholder={'mcr.microsoft.com/devcontainers/*\nghcr.io/acme/*'} />
              <div className="hint">* matches anything. Empty: any image a project names.</div>
            </label>
            <div>
              <label className="field">
                <span>Image for projects without a devcontainer.json</span>
                <input className="input mono" value={f.defaultImage} onChange={(e) => set({ defaultImage: e.target.value })} />
              </label>
              <label className="field">
                <span>Seconds a command may run</span>
                <input className="input" inputMode="numeric" value={f.commandTimeoutSecs} onChange={(e) => set({ commandTimeoutSecs: e.target.value })} />
              </label>
            </div>
          </div>
          <div className="two-col">
            {approval('commands and file changes')}
            <label className="check" style={{ alignSelf: 'center' }}>
              <input type="checkbox" checked={f.network} onChange={(e) => set({ network: e.target.checked })} /> Containers may reach the network (package installs, git)
            </label>
          </div>
        </>
      )}
      {f.access !== 'nobody' && k === 'browser' && (
        <>
          <div className="two-col">
            <label className="field">
              <span>Only these sites (one per line)</span>
              <textarea className="input mono" rows={3} value={f.allowedDomains} onChange={(e) => set({ allowedDomains: e.target.value })} placeholder="example.com" />
              <div className="hint">Subdomains included. Empty: every site that isn't blocked.</div>
            </label>
            <label className="field">
              <span>Blocked sites (one per line)</span>
              <textarea className="input mono" rows={3} value={f.blockedDomains} onChange={(e) => set({ blockedDomains: e.target.value })} />
            </label>
          </div>
          <div className="two-col">{approval('clicks and typing')}</div>
        </>
      )}
      <ErrorText error={save.error} />
      <div className="actions">
        <button className="btn primary" disabled={!dirty || save.isPending} onClick={() => save.mutate()}>{save.isPending ? 'Saving…' : dirty ? 'Save' : 'Saved'}</button>
      </div>
    </div>
  )
}

const pairs = (o: Record<string, string> | undefined, sep: string) => Object.entries(o ?? {}).map(([k, v]) => `${k}${sep}${v}`).join('\n')

function parsePairs(s: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const l of lines(s)) {
    const i = l.indexOf(sep)
    if (i > 0) out[l.slice(0, i).trim()] = l.slice(i + 1).trim()
  }
  return out
}

function ConnectorDialog({ c, groups, onClose }: { c: LocalConnector | null; groups: Group[]; onClose: () => void }) {
  const qc = useQueryClient()
  const [f, setF] = useState({
    name: c?.name ?? '',
    description: c?.description ?? '',
    kind: c?.kind ?? ('mcp_stdio' as LocalConnector['kind']),
    command: c?.config.command ?? '',
    args: (c?.config.args ?? []).join('\n'),
    env: pairs(c?.config.env, '='),
    url: c?.config.url ?? 'http://127.0.0.1:3000/mcp',
    headers: pairs(c?.config.headers, ': '),
    tools: (c?.tools ?? []).join('\n'),
    approval: c?.approval ?? ('ask' as LocalConnector['approval']),
    enabled: c?.enabled ?? true,
    access: c?.access ?? ('groups' as LocalConnector['access']),
    groupIds: new Set(c?.groupIds ?? []),
  })
  const done = () => {
    qc.invalidateQueries({ queryKey: ['admin-desktop'] })
    onClose()
  }
  const save = useMutation({
    mutationFn: () => {
      const body = {
        name: f.name.trim(),
        description: f.description,
        kind: f.kind,
        // args keep their inner spaces: one argument per line
        config:
          f.kind === 'mcp_stdio'
            ? { command: f.command.trim(), args: f.args.split('\n').filter((l) => l.trim()), env: parsePairs(f.env, '=') }
            : { url: f.url.trim(), headers: parsePairs(f.headers, ':') },
        tools: lines(f.tools),
        approval: f.approval,
        enabled: f.enabled,
        access: f.access,
        groupIds: [...f.groupIds],
      }
      return c ? api.patch(`/admin/desktop/connectors/${c.id}`, body) : api.post('/admin/desktop/connectors', body)
    },
    onSuccess: done,
  })
  const remove = useMutation({ mutationFn: () => api.del(`/admin/desktop/connectors/${c!.id}`), onSuccess: done })
  const ready = f.name.trim() && (f.kind === 'mcp_stdio' ? f.command.trim() : f.url.trim())
  return (
    <Dialog title={c ? `Edit ${c.name}` : 'Add a local connector'} onClose={onClose} wide>
      <div className="two-col">
        <label className="field"><span>Name (shown to people and the model)</span><input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoFocus={!c} /></label>
        <label className="field">
          <span>Kind</span>
          <select className="input" value={f.kind} onChange={(e) => setF({ ...f, kind: e.target.value as LocalConnector['kind'] })}>
            <option value="mcp_stdio">MCP server the app starts (a program)</option>
            <option value="mcp_http">MCP server at a URL reachable from the computer</option>
          </select>
        </label>
      </div>
      <label className="field">
        <span>What it's for</span>
        <input className="input" value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} placeholder="e.g. the local Outlook mailbox, files under ~/Documents, the test database" />
        <div className="hint">Helps the model decide when to use it.</div>
      </label>
      {f.kind === 'mcp_stdio' ? (
        <>
          <label className="field">
            <span>Command</span>
            <input className="input mono" value={f.command} onChange={(e) => setF({ ...f, command: e.target.value })} placeholder="npx" />
            <div className="hint">Runs on the person's computer, as them. It has to be installed there.</div>
          </label>
          <div className="two-col">
            <label className="field"><span>Arguments (one per line)</span><textarea className="input mono" rows={4} value={f.args} onChange={(e) => setF({ ...f, args: e.target.value })} placeholder={'-y\n@modelcontextprotocol/server-filesystem\n~/Documents'} /></label>
            <label className="field"><span>Environment (KEY=VALUE per line)</span><textarea className="input mono" rows={4} value={f.env} onChange={(e) => setF({ ...f, env: e.target.value })} /></label>
          </div>
        </>
      ) : (
        <>
          <label className="field"><span>URL</span><input className="input mono" value={f.url} onChange={(e) => setF({ ...f, url: e.target.value })} /></label>
          <label className="field"><span>Headers (Name: value per line)</span><textarea className="input mono" rows={3} value={f.headers} onChange={(e) => setF({ ...f, headers: e.target.value })} /></label>
        </>
      )}
      <div className="hint" style={{ marginTop: -6, marginBottom: 12 }}>These settings are sent to the apps of the people who may use the connector: don't put secrets here that they shouldn't see.</div>
      <div className="two-col">
        <label className="field">
          <span>Only these tools (one per line)</span>
          <textarea className="input mono" rows={3} value={f.tools} onChange={(e) => setF({ ...f, tools: e.target.value })} />
          <div className="hint">Names as the server lists them. Empty: all its tools.</div>
        </label>
        <div>
          <label className="field">
            <span>Before a tool runs</span>
            <select className="input" value={f.approval} onChange={(e) => setF({ ...f, approval: e.target.value as LocalConnector['approval'] })}>
              <option value="ask">The person confirms each call in the app</option>
              <option value="auto">Run without asking</option>
            </select>
          </label>
          <label className="check"><input type="checkbox" checked={f.enabled} onChange={(e) => setF({ ...f, enabled: e.target.checked })} /> Enabled</label>
        </div>
      </div>
      <h3>Who may use it</h3>
      <AccessPicker
        name="connector-access"
        access={f.access}
        options={[['all', 'Everyone with Agora Desktop'], ['groups', 'Only these groups']]}
        groupIds={f.groupIds}
        groups={groups}
        onChange={(access, groupIds) => setF({ ...f, access, groupIds })}
      />
      <ErrorText error={save.error ?? remove.error} />
      <div className="actions">
        {c && <button className="btn danger" style={{ marginRight: 'auto' }} onClick={() => confirm(`Remove ${c.name}?`) && remove.mutate()}>Remove</button>}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!ready || save.isPending} onClick={() => save.mutate()}>Save</button>
      </div>
    </Dialog>
  )
}

function Devices({ devices }: { devices: Device[] }) {
  const qc = useQueryClient()
  const revoke = useMutation({ mutationFn: (id: string) => api.del(`/admin/desktop/devices/${id}`), onSuccess: () => qc.invalidateQueries({ queryKey: ['admin-desktop'] }) })
  return (
    <>
      <div className="table-wrap">
        <table className="data">
          <thead><tr><th>Person</th><th>Computer</th><th>System</th><th>Version</th><th>Last seen</th><th /></tr></thead>
          <tbody>
            {devices.map((d) => (
              <tr key={d.id}>
                <td>{d.user ?? '—'}</td>
                <td>
                  <strong>{d.name}</strong>{' '}
                  {d.revokedAt ? <span className="badge danger">signed out {ago(d.revokedAt)}</span> : d.connected && <span className="badge accent">connected</span>}
                </td>
                <td>{d.platform ?? ''}</td>
                <td>{d.appVersion ?? ''}</td>
                <td style={{ whiteSpace: 'nowrap' }}>{d.lastSeenAt ? ago(d.lastSeenAt) : 'never'}</td>
                <td style={{ textAlign: 'right' }}>
                  {!d.revokedAt && (
                    <button className="btn small ghost" disabled={revoke.isPending} onClick={() => confirm(`Sign ${d.name} (${d.user ?? 'unknown'}) out?`) && revoke.mutate(d.id)}>Sign out</button>
                  )}
                </td>
              </tr>
            ))}
            {devices.length === 0 && <tr><td colSpan={6} className="empty">No one has signed in with Agora Desktop yet.</td></tr>}
          </tbody>
        </table>
      </div>
      <ErrorText error={revoke.error} />
    </>
  )
}

interface ToolEvent {
  id: string
  at: string
  userId: string | null
  user: string | null
  chatId: string | null
  device: string | null
  feature: 'code' | 'browser' | 'connector'
  connector: string | null
  tool: string
  input: Record<string, unknown>
  status: 'ok' | 'error' | 'denied' | 'blocked' | 'timeout'
  error: string | null
  resultChars: number
  durationMs: number | null
}

/** The gist of a call's input for the table row. */
function gist(e: ToolEvent): string {
  const i = e.input ?? {}
  const v = i.command ?? i.path ?? i.url ?? i.pattern ?? i.query ?? i.text
  if (typeof v === 'string') return v
  const s = JSON.stringify(i)
  return s === '{}' ? '' : s
}

function ToolActivity() {
  const [feature, setFeature] = useState('')
  const [q, setQ] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  const params = new URLSearchParams()
  if (feature) params.set('feature', feature)
  if (q.trim()) params.set('q', q.trim())
  const events = useInfiniteQuery({
    queryKey: ['tool-events', params.toString()],
    queryFn: ({ pageParam }) => api.get<{ events: ToolEvent[]; more: boolean }>(`/admin/desktop/tools?${params}${pageParam ? `&before=${encodeURIComponent(pageParam)}` : ''}`),
    initialPageParam: '',
    getNextPageParam: (last) => (last.more ? last.events[last.events.length - 1]!.at : undefined),
  })
  const rows = events.data?.pages.flatMap((p) => p.events) ?? []
  return (
    <>
      <p className="muted" style={{ marginTop: 0 }}>Every tool a model ran on someone's computer: for whom, on which device, with what input, and how it ended.</p>
      <div className="row" style={{ marginBottom: 12 }}>
        <select className="input" style={{ width: 'auto' }} value={feature} onChange={(e) => setFeature(e.target.value)} aria-label="Feature">
          <option value="">Everything</option>
          <option value="code">Code (dev containers)</option>
          <option value="browser">Browser</option>
          <option value="connector">Local connectors</option>
        </select>
        <input className="input" style={{ maxWidth: 320 }} placeholder="Search tools, commands, paths, URLs" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search" />
      </div>
      <ErrorText error={events.error} />
      <div className="table-wrap">
        <table className="data">
          <thead><tr><th>Time</th><th>Person</th><th>Device</th><th>Tool</th><th>Input</th><th>Status</th></tr></thead>
          <tbody>
            {rows.map((e) => (
              <Fragment key={e.id}>
                <tr className="clickable" onClick={() => setOpen(open === e.id ? null : e.id)}>
                  <td style={{ whiteSpace: 'nowrap' }}>{when(e.at)}</td>
                  <td>{e.user ?? '—'}</td>
                  <td>{e.device ?? '—'}</td>
                  <td style={{ whiteSpace: 'nowrap' }}><span className="badge">{e.connector ?? e.feature}</span> <span className="mono">{e.tool}</span></td>
                  <td className="mono" style={{ maxWidth: 380, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{gist(e)}</td>
                  <td style={{ whiteSpace: 'nowrap' }}>
                    <span className={`badge${e.status === 'ok' ? '' : e.status === 'error' || e.status === 'timeout' ? ' danger' : ' warn'}`}>{e.status}</span>
                    {e.durationMs != null && <span className="faint" style={{ fontSize: 12, marginLeft: 4 }}>{(e.durationMs / 1000).toFixed(1)} s</span>}
                  </td>
                </tr>
                {open === e.id && (
                  <tr>
                    <td colSpan={6}>
                      <pre className="json">{JSON.stringify(e.input, null, 2)}</pre>
                      {e.error && <div className="error-text" style={{ marginTop: 6, whiteSpace: 'pre-wrap' }}>{e.error}</div>}
                      <div className="faint" style={{ marginTop: 6, fontSize: 13 }}>
                        Result: {e.resultChars.toLocaleString()} characters{e.chatId && <> · <Link to={`/admin/chats/${e.chatId}`}>Open the conversation (recorded)</Link></>}
                      </div>
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
            {rows.length === 0 && !events.isLoading && <tr><td colSpan={6} className="empty">No tool activity yet.</td></tr>}
          </tbody>
        </table>
      </div>
      {events.hasNextPage && <div className="pager"><button className="btn" onClick={() => events.fetchNextPage()}>Older</button></div>}
    </>
  )
}

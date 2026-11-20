import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, plural } from '../../api'
import { Dialog, ErrorText } from '../../ui'
import { AccessPicker, lines, parsePairs, ToolActivity, useGroups, type Group } from './Desktop'

interface McpServer {
  id: string
  name: string
  kind: 'mcp_stdio' | 'mcp_http'
  description: string | null
  config: { command?: string; args?: string[]; url?: string }
  /** names of the stored environment variables and headers (their values are never shown) */
  envKeys: string[]
  headerKeys: string[]
  tools: string[]
  approval: 'ask' | 'auto'
  enabled: boolean
  access: 'all' | 'groups'
  groupIds: string[]
}

interface RemoteTool {
  name: string
  description: string
  readOnly: boolean
}

/** Admin → MCP servers: tools of the organization's services, run by the server, in every chat. */
export function McpAdmin({ admin }: { admin: boolean }) {
  const list = useQuery({ queryKey: ['admin-mcp'], queryFn: () => api.get<McpServer[]>('/admin/mcp-servers'), enabled: admin })
  const groups = useGroups()
  const [editing, setEditing] = useState<McpServer | 'new' | null>(null)
  return (
    <>
      {admin && (
        <>
          <div className="row">
            <h2 className="grow">MCP servers</h2>
            <button className="btn" onClick={() => setEditing('new')}>Add a server</button>
          </div>
          <p className="muted" style={{ marginTop: 0 }}>
            Model Context Protocol servers the Agora server connects to: ticket systems, CRMs, databases, internal APIs. Their tools are offered to models in
            every chat (the web app included) of the people you grant them to, who switch them on and off in the chat. Every call is recorded. For servers on
            people's own computers, use local connectors under Desktop.
          </p>
          <ErrorText error={list.error} />
          {(list.data?.length ?? 0) > 0 && (
            <div className="table-wrap">
              <table className="data">
                <tbody>
                  {list.data!.map((c) => (
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
                      <td className="faint">{c.tools.length ? `${plural(c.tools.length, 'tool')} allowed` : 'all its tools'}</td>
                      <td style={{ textAlign: 'right' }}><button className="btn small ghost" onClick={() => setEditing(c)}>Edit</button></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {list.data?.length === 0 && <p className="faint">No MCP servers yet.</p>}
        </>
      )}
      <h2 style={{ marginTop: admin ? 24 : 0 }}>Tool activity</h2>
      <ToolActivity only="mcp" />
      {editing && <ServerDialog c={editing === 'new' ? null : editing} groups={groups.data ?? []} onClose={() => setEditing(null)} />}
    </>
  )
}

/** Stored secrets: their names, and a way to replace them (values are never sent back). */
function SecretField(props: { label: string; stored: string[]; value: string | null; onChange: (v: string | null) => void; placeholder: string; sep: string }) {
  const replacing = props.value !== null
  return (
    <label className="field">
      <span>{props.label}</span>
      {replacing ? (
        <textarea className="input mono" rows={3} value={props.value!} onChange={(e) => props.onChange(e.target.value)} placeholder={props.placeholder} />
      ) : (
        <div className="row" style={{ gap: 8 }}>
          <span className="faint grow mono" style={{ overflowWrap: 'anywhere' }}>{props.stored.length ? props.stored.map((k) => `${k}${props.sep}••••`).join(', ') : 'none'}</span>
          <button type="button" className="btn small" onClick={() => props.onChange('')}>{props.stored.length ? 'Replace' : 'Add'}</button>
        </div>
      )}
      <div className="hint">Stored encrypted; never shown again.{replacing && props.stored.length > 0 && ' What you enter replaces all stored values.'}</div>
    </label>
  )
}

function ServerDialog({ c, groups, onClose }: { c: McpServer | null; groups: Group[]; onClose: () => void }) {
  const qc = useQueryClient()
  const [f, setF] = useState({
    name: c?.name ?? '',
    description: c?.description ?? '',
    kind: c?.kind ?? ('mcp_http' as McpServer['kind']),
    command: c?.config.command ?? '',
    args: (c?.config.args ?? []).join('\n'),
    url: c?.config.url ?? '',
    // null: keep what is stored
    env: c ? null : ('' as string | null),
    headers: c ? null : ('' as string | null),
    some: (c?.tools.length ?? 0) > 0,
    tools: new Set(c?.tools ?? []),
    toolText: (c?.tools ?? []).join('\n'),
    approval: c?.approval ?? ('ask' as McpServer['approval']),
    enabled: c?.enabled ?? true,
    access: c?.access ?? ('groups' as McpServer['access']),
    groupIds: new Set(c?.groupIds ?? []),
  })
  const [found, setFound] = useState<RemoteTool[] | null>(null)
  const settings = () => ({
    name: f.name.trim(),
    kind: f.kind,
    // args keep their inner spaces: one argument per line
    config: f.kind === 'mcp_stdio' ? { command: f.command.trim(), args: f.args.split('\n').filter((l) => l.trim()) } : { url: f.url.trim() },
    ...(f.kind === 'mcp_stdio' ? (f.env !== null ? { env: parsePairs(f.env, '=') } : {}) : f.headers !== null ? { headers: parsePairs(f.headers, ':') } : {}),
  })
  const allowed = () => (!f.some ? [] : found ? [...f.tools].filter((t) => found.some((x) => x.name === t)) : lines(f.toolText))
  const done = () => {
    qc.invalidateQueries({ queryKey: ['admin-mcp'] })
    qc.invalidateQueries({ queryKey: ['mcp-servers'] })
    onClose()
  }
  const check = useMutation({
    mutationFn: () => api.post<{ ok: boolean; tools?: RemoteTool[]; error?: string }>('/admin/mcp-servers/check', { ...settings(), id: c?.id }),
    onSuccess: (r) => {
      if (!r.ok) return
      setFound(r.tools ?? [])
      // what was typed before the list was known stays chosen
      if (f.some && f.tools.size === 0) setF({ ...f, tools: new Set(lines(f.toolText)) })
    },
  })
  const save = useMutation({
    mutationFn: () => {
      const body = {
        ...settings(),
        description: f.description,
        tools: allowed(),
        approval: f.approval,
        enabled: f.enabled,
        access: f.access,
        groupIds: [...f.groupIds],
      }
      return c ? api.patch(`/admin/mcp-servers/${c.id}`, body) : api.post('/admin/mcp-servers', body)
    },
    onSuccess: done,
  })
  const remove = useMutation({ mutationFn: () => api.del(`/admin/mcp-servers/${c!.id}`), onSuccess: done })
  const ready = f.name.trim() && (f.kind === 'mcp_stdio' ? f.command.trim() : f.url.trim()) && !(f.some && allowed().length === 0)
  return (
    <Dialog title={c ? `Edit ${c.name}` : 'Add an MCP server'} onClose={onClose} wide>
      <div className="two-col">
        <label className="field"><span>Name (shown to people and the model)</span><input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoFocus={!c} /></label>
        <label className="field">
          <span>Kind</span>
          <select className="input" value={f.kind} onChange={(e) => setF({ ...f, kind: e.target.value as McpServer['kind'] })}>
            <option value="mcp_http">MCP server at a URL</option>
            <option value="mcp_stdio">MCP server the Agora server starts (a program)</option>
          </select>
        </label>
      </div>
      <label className="field">
        <span>What it's for</span>
        <input className="input" value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} placeholder="e.g. tickets in Jira, the CRM's customers and deals, the reporting database (read-only)" />
        <div className="hint">Helps the model decide when to use it.</div>
      </label>
      {f.kind === 'mcp_stdio' ? (
        <>
          <label className="field">
            <span>Command</span>
            <input className="input mono" value={f.command} onChange={(e) => setF({ ...f, command: e.target.value })} placeholder="npx" />
            <div className="hint">Runs on the Agora server, as its user, in a folder of its own, with none of the server's environment except PATH and HOME. It has to be installed there.</div>
          </label>
          <div className="two-col">
            <label className="field"><span>Arguments (one per line)</span><textarea className="input mono" rows={3} value={f.args} onChange={(e) => setF({ ...f, args: e.target.value })} placeholder={'-y\n@modelcontextprotocol/server-github'} /></label>
            <SecretField label="Environment (KEY=VALUE per line)" stored={c?.envKeys ?? []} value={f.env} onChange={(env) => setF({ ...f, env })} placeholder="GITHUB_TOKEN=…" sep="=" />
          </div>
        </>
      ) : (
        <div className="two-col">
          <label className="field">
            <span>URL</span>
            <input className="input mono" value={f.url} onChange={(e) => setF({ ...f, url: e.target.value })} placeholder="https://mcp.example.com/mcp" />
            <div className="hint">Streamable HTTP. Reached from the Agora server.</div>
          </label>
          <SecretField label="Headers (Name: value per line)" stored={c?.headerKeys ?? []} value={f.headers} onChange={(headers) => setF({ ...f, headers })} placeholder="Authorization: Bearer …" sep=": " />
        </div>
      )}
      <div className="hint" style={{ marginTop: -6, marginBottom: 12 }}>
        Everyone who may use the server acts with these credentials: give it an account with only the access those people should have.
      </div>

      <h3>Tools</h3>
      <div className="row" style={{ gap: 10, marginBottom: 10 }}>
        <button className="btn" disabled={check.isPending || !(f.kind === 'mcp_stdio' ? f.command.trim() : f.url.trim())} onClick={() => check.mutate()}>
          {check.isPending ? 'Connecting…' : found ? 'Check again' : 'Check the connection'}
        </button>
        {found && <span className="badge accent">connected: {found.length} tools</span>}
        {check.data && !check.data.ok && <span className="error-text" style={{ fontSize: 13 }}>{check.data.error}</span>}
        <ErrorText error={check.error} />
      </div>
      <div className="row" style={{ gap: 18 }}>
        <label className="check"><input type="radio" name="mcp-some" checked={!f.some} onChange={() => setF({ ...f, some: false })} /> All its tools (also ones it adds later)</label>
        <label className="check"><input type="radio" name="mcp-some" checked={f.some} onChange={() => setF({ ...f, some: true })} /> Only these</label>
      </div>
      {f.some &&
        (found ? (
          <div style={{ maxHeight: 260, overflowY: 'auto', marginBottom: 12 }}>
            {found.map((t) => (
              <label key={t.name} className="check" style={{ marginBottom: 6 }}>
                <input
                  type="checkbox"
                  checked={f.tools.has(t.name)}
                  onChange={(e) => {
                    const s = new Set(f.tools)
                    if (e.target.checked) s.add(t.name)
                    else s.delete(t.name)
                    setF({ ...f, tools: s })
                  }}
                />
                <span>
                  <span className="mono">{t.name}</span> {t.readOnly && <span className="badge">reads only</span>}
                  {t.description && <div className="faint" style={{ fontSize: 13 }}>{t.description.length > 220 ? `${t.description.slice(0, 220)}…` : t.description}</div>}
                </span>
              </label>
            ))}
            {found.length === 0 && <span className="faint">The server lists no tools.</span>}
          </div>
        ) : (
          <label className="field">
            <span>Tool names (one per line)</span>
            <textarea className="input mono" rows={3} value={f.toolText} onChange={(e) => setF({ ...f, toolText: e.target.value })} />
            <div className="hint">As the server lists them. Check the connection to pick them from a list.</div>
          </label>
        ))}
      <div className="two-col">
        <label className="field">
          <span>Before a tool runs</span>
          <select className="input" value={f.approval} onChange={(e) => setF({ ...f, approval: e.target.value as McpServer['approval'] })}>
            <option value="ask">The person allows each call in the chat</option>
            <option value="auto">Run without asking</option>
          </select>
        </label>
        <label className="check" style={{ alignSelf: 'center' }}><input type="checkbox" checked={f.enabled} onChange={(e) => setF({ ...f, enabled: e.target.checked })} /> Enabled</label>
      </div>
      <h3>Who may use it</h3>
      <AccessPicker
        name="mcp-access"
        access={f.access}
        options={[['all', 'Everyone'], ['groups', 'Only these groups']]}
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

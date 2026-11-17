import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useLocation, useNavigate, useParams } from 'react-router-dom'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  ArrowUp,
  Brain,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FileText,
  Globe,
  Pencil,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Square,
  TriangleAlert,
} from 'lucide-react'
import { api, ApiError, num, streamEvents, usd, type Chat, type Me, type Message, type ModelInfo, type Part, type StreamEvent } from '../api'
import { CopyButton, Dialog, hostOf, Markdown } from '../ui'

interface ChatData {
  chat: Chat
  messages: Message[]
  live: string[]
}

interface Pending {
  content: string
  modelId: string
  webSearch: boolean
  effort: string | null
}

const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max']

export function ChatPage({ me }: { me: Me }) {
  const { id } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const qc = useQueryClient()
  const models = useQuery({ queryKey: ['models'], queryFn: () => api.get<ModelInfo[]>('/models') })
  const data = useQuery({ queryKey: ['chat', id], queryFn: () => api.get<ChatData>(`/chats/${id}`), enabled: !!id, staleTime: Infinity, gcTime: 0 })

  const [messages, setMessages] = useState<Record<string, Message>>({})
  const [leafId, setLeafId] = useState<string | null>(null)
  const [title, setTitle] = useState<string>('New chat')
  const [modelId, setModelId] = useState<string | null>(null)
  const [webSearch, setWebSearch] = useState(false)
  const [effort, setEffort] = useState<string | null>(null)
  const [systemPrompt, setSystemPrompt] = useState<string>('')
  const [streamingId, setStreamingId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [showSettings, setShowSettings] = useState(false)
  const loadedFor = useRef<string | null>(null)

  // load the chat into local state (once per chat)
  useEffect(() => {
    if (!data.data || loadedFor.current === data.data.chat.id) return
    loadedFor.current = data.data.chat.id
    const c = data.data.chat
    setMessages(Object.fromEntries(data.data.messages.map((m) => [m.id, m])))
    setLeafId(c.leafId)
    setTitle(c.title)
    setModelId(c.modelId)
    setWebSearch(!!c.settings.webSearch)
    setEffort(c.settings.effort ?? null)
    setSystemPrompt(c.systemPrompt ?? '')
    for (const live of data.data.live) follow(live)
  }, [data.data]) // eslint-disable-line react-hooks/exhaustive-deps

  const modelList = models.data ?? []
  const model = modelList.find((m) => m.id === modelId) ?? modelList.find((m) => m.id === me.defaultModelId) ?? modelList[0]
  const effortShown = model?.kind === 'anthropic' && model.reasoning

  const applyEvent = useCallback((ev: StreamEvent, targetId: { current: string | null }) => {
    if (ev.type === 'start') {
      targetId.current = ev.assistantMessage.id
      setStreamingId(ev.assistantMessage.id)
      setMessages((ms) => ({ ...ms, [ev.userMessage.id]: ev.userMessage, [ev.assistantMessage.id]: ev.assistantMessage }))
      setLeafId(ev.assistantMessage.id)
      return
    }
    if (ev.type === 'title') {
      setTitle(ev.title)
      qc.invalidateQueries({ queryKey: ['chats'] })
      return
    }
    const tid = targetId.current
    if (!tid) return
    setMessages((ms) => {
      const m = ms[tid]
      if (!m) return ms
      switch (ev.type) {
        case 'snapshot':
          return { ...ms, [tid]: { ...m, content: ev.content, parts: ev.parts } }
        case 'text':
          return { ...ms, [tid]: { ...m, content: m.content + ev.d } }
        case 'reasoning': {
          const parts = [...m.parts]
          const last = parts[parts.length - 1]
          if (last?.type === 'reasoning') parts[parts.length - 1] = { ...last, text: last.text + ev.d }
          else parts.push({ type: 'reasoning', text: ev.d })
          return { ...ms, [tid]: { ...m, parts } }
        }
        case 'part': {
          const p = ev.part as Part & { id: string }
          const parts = [...m.parts]
          const i = parts.findIndex((x) => x.type === p.type && (x as { id?: string }).id === p.id)
          if (i >= 0) parts[i] = p
          else parts.push(p)
          return { ...ms, [tid]: { ...m, parts } }
        }
        case 'done':
          return { ...ms, [tid]: ev.message }
        default:
          return ms
      }
    })
    if (ev.type === 'done') qc.invalidateQueries({ queryKey: ['chats'] })
  }, [qc])

  async function run(path: string, body: unknown | undefined, knownId?: string) {
    const target = { current: knownId ?? null }
    if (knownId) setStreamingId(knownId)
    setError(null)
    try {
      await streamEvents(path, body, (ev) => applyEvent(ev, target))
    } catch (e) {
      setError(e instanceof ApiError ? e.message : `The connection broke off: ${e instanceof Error ? e.message : e}`)
    } finally {
      setStreamingId(null)
    }
  }

  function follow(messageId: string) {
    run(`/messages/${messageId}/stream`, undefined, messageId)
  }

  async function send(content: string, parentId: string | null) {
    if (!model) return setError('No model is available to you yet. Ask an administrator to connect one.')
    const body = { content, parentId, modelId: model.id, webSearch: webSearch && model.webSearch, effort: effortShown ? effort : null }
    if (!id) {
      try {
        const chat = await api.post<Chat>('/chats', { modelId: model.id, systemPrompt: systemPrompt || null, settings: { webSearch, effort } })
        qc.invalidateQueries({ queryKey: ['chats'] })
        navigate(`/c/${chat.id}`, { state: { pending: { content, modelId: model.id, webSearch: body.webSearch, effort: body.effort } satisfies Pending } })
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e))
      }
      return
    }
    await run(`/chats/${id}/messages`, body)
  }

  // a message typed on the start page, sent once the chat exists
  useEffect(() => {
    const pending = (location.state as { pending?: Pending } | null)?.pending
    if (!id || !pending || !data.data || loadedFor.current !== id) return
    navigate(location.pathname, { replace: true, state: null })
    run(`/chats/${id}/messages`, { ...pending, parentId: null })
  }, [id, data.data, location.state]) // eslint-disable-line react-hooks/exhaustive-deps

  async function regenerate(assistant: Message) {
    if (!id || !assistant.parentId) return
    await run(`/chats/${id}/regenerate`, { messageId: assistant.parentId, modelId: model?.id, webSearch: webSearch && model?.webSearch, effort: effortShown ? effort : null })
  }

  function stop() {
    if (streamingId) api.post(`/messages/${streamingId}/stop`).catch(() => {})
  }

  // the branch being shown: root → leaf
  const path = useMemo(() => {
    const out: Message[] = []
    let cur = leafId ? messages[leafId] : undefined
    const seen = new Set<string>()
    while (cur && !seen.has(cur.id)) {
      seen.add(cur.id)
      out.unshift(cur)
      cur = cur.parentId ? messages[cur.parentId] : undefined
    }
    return out
  }, [messages, leafId])

  const children = useMemo(() => {
    const map: Record<string, Message[]> = {}
    for (const m of Object.values(messages)) (map[m.parentId ?? 'root'] ??= []).push(m)
    for (const k in map) map[k]!.sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id))
    return map
  }, [messages])

  function switchBranch(to: Message) {
    // follow the newest answers down from the chosen message
    let cur = to
    for (;;) {
      const kids = children[cur.id]
      if (!kids?.length) break
      cur = kids[kids.length - 1]!
    }
    setLeafId(cur.id)
    if (id) api.patch(`/chats/${id}`, { leafId: cur.id }).catch(() => {})
  }

  // keep the newest text in view while it streams, unless the reader scrolled up
  const threadRef = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  useLayoutEffect(() => {
    const el = threadRef.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  }, [path])

  const lastAssistant = [...path].reverse().find((m) => m.role === 'assistant')
  const busy = !!streamingId || lastAssistant?.status === 'streaming'
  const lastId = path[path.length - 1]?.id ?? null

  if (id && data.isError) {
    return <div className="empty-state"><div className="inner"><h1>Chat not found</h1><p className="muted">It may have been deleted.</p></div></div>
  }

  return (
    <>
      <header className="chat-header">
        <ModelPicker models={modelList} value={model} onChange={(m) => {
          setModelId(m.id)
          if (id) api.patch(`/chats/${id}`, { modelId: m.id }).catch(() => {})
        }} />
        <div className="grow chat-title" title={title}>{id ? title : ''}</div>
        {me.allowUserSystemPrompt && (
          <button className={`icon-btn`} title="Chat instructions" aria-label="Chat instructions" onClick={() => setShowSettings(true)}>
            <SlidersHorizontal size={17} />
          </button>
        )}
      </header>
      {path.length === 0 && !id ? (
        <div className="empty-state">
          <div className="inner">
            <h1>How can I help, {me.user!.name.split(' ')[0]}?</h1>
            <Composer
              me={me}
              model={model}
              webSearch={webSearch}
              setWebSearch={setWebSearch}
              effort={effort}
              setEffort={setEffort}
              effortShown={!!effortShown}
              busy={busy}
              onSend={(t) => send(t, null)}
              onStop={stop}
              autoFocus
            />
            {modelList.length === 0 && models.isSuccess && (
              <div className="banner" style={{ marginTop: 16 }}>No models are available to you yet. An administrator connects them under Administration → Models.</div>
            )}
          </div>
        </div>
      ) : (
        <>
          <div
            className="thread"
            ref={threadRef}
            onScroll={(e) => {
              const el = e.currentTarget
              stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
            }}
          >
            <div className="thread-inner">
              {path.map((m) => {
                const siblings = children[m.parentId ?? 'root']?.filter((s) => s.role === m.role) ?? [m]
                return m.role === 'user' ? (
                  <UserMessage
                    key={m.id}
                    m={m}
                    siblings={siblings}
                    onSwitch={switchBranch}
                    busy={busy}
                    onEdit={(text) => {
                      stick.current = true
                      send(text, m.parentId)
                    }}
                  />
                ) : (
                  <AssistantMessage
                    key={m.id}
                    m={m}
                    siblings={siblings}
                    onSwitch={switchBranch}
                    streaming={m.id === streamingId || m.status === 'streaming'}
                    canRegenerate={!busy && m.id === lastId}
                    onRegenerate={() => {
                      stick.current = true
                      regenerate(m)
                    }}
                  />
                )
              })}
              {error && <div className="banner error">{error}</div>}
            </div>
          </div>
          <div className="composer-wrap">
            <Composer
              me={me}
              model={model}
              webSearch={webSearch}
              setWebSearch={setWebSearch}
              effort={effort}
              setEffort={setEffort}
              effortShown={!!effortShown}
              busy={busy}
              onSend={(t) => {
                stick.current = true
                send(t, lastId)
              }}
              onStop={stop}
            />
          </div>
        </>
      )}
      {showSettings && (
        <ChatSettings
          initial={systemPrompt}
          onClose={() => setShowSettings(false)}
          onSave={(s) => {
            setSystemPrompt(s)
            if (id) api.patch(`/chats/${id}`, { systemPrompt: s }).catch(() => {})
            setShowSettings(false)
          }}
        />
      )}
    </>
  )
}

function ModelPicker({ models, value, onChange }: { models: ModelInfo[]; value?: ModelInfo; onChange: (m: ModelInfo) => void }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => ref.current && !ref.current.contains(e.target as Node) && setOpen(false)
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])
  return (
    <div className="model-picker" ref={ref}>
      <button className="btn ghost" onClick={() => setOpen(!open)} aria-haspopup="listbox" aria-expanded={open}>
        {value?.name ?? 'No model'} <ChevronDown size={15} />
      </button>
      {open && (
        <div className="menu" role="listbox">
          {models.map((m) => (
            <button
              key={m.id}
              role="option"
              aria-selected={m.id === value?.id}
              className={`menu-item${m.id === value?.id ? ' selected' : ''}`}
              onClick={() => {
                onChange(m)
                setOpen(false)
              }}
            >
              <div className="grow">
                <div style={{ fontWeight: 600 }}>{m.name}</div>
                <div className="sub">
                  {m.description || m.provider}
                  {m.inputPrice > 0 && ` · ${usd(m.inputPrice, 2)} / ${usd(m.outputPrice, 2)} per M tokens`}
                </div>
              </div>
              <div className="row" style={{ gap: 4 }}>
                {m.webSearch && <span className="badge" title="Can search the web"><Globe size={12} /></span>}
                {m.reasoning && <span className="badge" title="Thinks before answering"><Brain size={12} /></span>}
              </div>
            </button>
          ))}
          {models.length === 0 && <div className="empty">No models available.</div>}
        </div>
      )}
    </div>
  )
}

function Composer(props: {
  me: Me
  model?: ModelInfo
  webSearch: boolean
  setWebSearch: (b: boolean) => void
  effort: string | null
  setEffort: (e: string | null) => void
  effortShown: boolean
  busy: boolean
  onSend: (text: string) => void
  onStop: () => void
  autoFocus?: boolean
}) {
  const [text, setText] = useState('')
  const ta = useRef<HTMLTextAreaElement>(null)
  useLayoutEffect(() => {
    const el = ta.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 260)}px`
  }, [text])
  const canSearch = !!props.model?.webSearch
  function submit() {
    if (!text.trim() || props.busy) return
    props.onSend(text)
    setText('')
  }
  return (
    <div>
      <div className="composer">
        <textarea
          ref={ta}
          rows={1}
          value={text}
          autoFocus={props.autoFocus}
          placeholder={props.model ? `Message ${props.model.name}` : 'Message'}
          aria-label="Message"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault()
              submit()
            }
          }}
        />
        <div className="bar">
          <button
            className={`toggle${props.webSearch && canSearch ? ' on' : ''}`}
            disabled={!canSearch}
            title={canSearch ? 'Let the model search the web (searches are logged)' : 'Web search is not available for this model'}
            aria-pressed={props.webSearch && canSearch}
            onClick={() => props.setWebSearch(!props.webSearch)}
          >
            <Globe size={14} /> Web search
          </button>
          {props.effortShown && (
            <select
              className="toggle"
              value={props.effort ?? ''}
              onChange={(e) => props.setEffort(e.target.value || null)}
              aria-label="How hard the model thinks"
              title="How hard the model thinks (more effort: better answers to hard questions, more tokens)"
            >
              <option value="">Effort: {props.model?.effort ?? 'default'}</option>
              {EFFORTS.map((e) => (
                <option key={e} value={e}>
                  Effort: {e}
                </option>
              ))}
            </select>
          )}
          <div className="grow" />
          {props.busy ? (
            <button className="send-btn" onClick={props.onStop} title="Stop" aria-label="Stop">
              <Square size={14} fill="currentColor" />
            </button>
          ) : (
            <button className="send-btn" onClick={submit} disabled={!text.trim() || !props.model} title="Send" aria-label="Send">
              <ArrowUp size={18} />
            </button>
          )}
        </div>
      </div>
      {props.me.notice && <div className="notice">{props.me.notice}</div>}
    </div>
  )
}

function BranchNav({ m, siblings, onSwitch }: { m: Message; siblings: Message[]; onSwitch: (m: Message) => void }) {
  if (siblings.length < 2) return null
  const i = siblings.findIndex((s) => s.id === m.id)
  return (
    <span className="branch-nav">
      <button className="icon-btn" disabled={i <= 0} onClick={() => onSwitch(siblings[i - 1]!)} aria-label="Previous version">
        <ChevronLeft size={15} />
      </button>
      {i + 1} / {siblings.length}
      <button className="icon-btn" disabled={i >= siblings.length - 1} onClick={() => onSwitch(siblings[i + 1]!)} aria-label="Next version">
        <ChevronRight size={15} />
      </button>
    </span>
  )
}

function UserMessage({ m, siblings, onSwitch, onEdit, busy }: { m: Message; siblings: Message[]; onSwitch: (m: Message) => void; onEdit: (t: string) => void; busy: boolean }) {
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState(m.content)
  if (editing) {
    return (
      <div className="msg user">
        <div className="msg-edit">
          <textarea className="input" value={text} onChange={(e) => setText(e.target.value)} autoFocus aria-label="Edit message" />
          <div className="row" style={{ justifyContent: 'flex-end', marginTop: 6 }}>
            <button className="btn small" onClick={() => setEditing(false)}>Cancel</button>
            <button
              className="btn small primary"
              disabled={!text.trim() || busy}
              onClick={() => {
                setEditing(false)
                onEdit(text)
              }}
            >
              Send
            </button>
          </div>
        </div>
      </div>
    )
  }
  return (
    <div className="msg user">
      <div className="bubble">{m.content}</div>
      <div className="tools">
        <BranchNav m={m} siblings={siblings} onSwitch={onSwitch} />
        <CopyButton text={m.content} />
        <button className="icon-btn" title="Edit" aria-label="Edit" disabled={busy} onClick={() => { setText(m.content); setEditing(true) }}>
          <Pencil size={15} />
        </button>
      </div>
    </div>
  )
}

function AssistantMessage(props: { m: Message; siblings: Message[]; onSwitch: (m: Message) => void; streaming: boolean; canRegenerate: boolean; onRegenerate: () => void }) {
  const { m, streaming } = props
  const waiting = streaming && !m.content && m.parts.length === 0
  return (
    <div className="msg assistant">
      <div className="assistant-head">{m.modelName ?? 'Assistant'}</div>
      {m.parts.length > 0 && (
        <div className="parts">
          {m.parts.map((p, i) => (
            <PartView key={i} p={p} live={streaming && i === m.parts.length - 1 && !m.content} />
          ))}
        </div>
      )}
      {waiting && (
        <div className="status-line">
          <span className="spinner" /> Thinking…
        </div>
      )}
      {m.content && (
        <div className={streaming ? 'streaming' : undefined}>
          <Markdown text={m.content + (streaming ? ' ▍' : '')} />
        </div>
      )}
      {m.status === 'error' && (
        <div className="banner error" style={{ marginTop: 8 }}>
          <TriangleAlert size={14} style={{ verticalAlign: -2 }} /> {m.error ?? 'The answer failed.'}
        </div>
      )}
      {m.status === 'stopped' && <div className="faint" style={{ fontSize: 13, marginTop: 4 }}>Stopped.</div>}
      {!streaming && (
        <div className="tools">
          <BranchNav m={m} siblings={props.siblings} onSwitch={props.onSwitch} />
          {m.content && <CopyButton text={m.content} />}
          {props.canRegenerate && (
            <button className="icon-btn" title="Answer again" aria-label="Answer again" onClick={props.onRegenerate}>
              <RefreshCw size={15} />
            </button>
          )}
          {(m.inputTokens > 0 || m.outputTokens > 0) && (
            <span className="meta" title={`${m.inputTokens.toLocaleString()} input tokens, ${m.outputTokens.toLocaleString()} output tokens`}>
              {num(m.inputTokens + m.outputTokens)} tokens · {usd(m.costUsd)}
              {m.durationMs ? ` · ${(m.durationMs / 1000).toFixed(1)} s` : ''}
            </span>
          )}
        </div>
      )}
    </div>
  )
}

function PartView({ p, live }: { p: Part; live: boolean }) {
  if (p.type === 'reasoning') {
    return (
      <details className="reasoning" open={live}>
        <summary>
          <Brain size={14} /> {live ? 'Thinking…' : 'Thought process'}
        </summary>
        <div className="reasoning-text">{p.text}</div>
      </details>
    )
  }
  if (p.type === 'search') {
    const n = p.results?.length ?? 0
    return (
      <details className="tool">
        <summary>
          {p.status === 'running' ? <span className="spinner" /> : <Search size={14} />}
          <span className="q">{p.query}</span>
          <span className="grow" />
          {p.status === 'done' && <span className="faint">{n} results</span>}
          {p.status === 'error' && <span className="error-text">{p.error ?? 'failed'}</span>}
        </summary>
        {n > 0 && (
          <div className="sources">
            {p.results!.map((r, i) => (
              <a key={i} className="source" href={r.url} target="_blank" rel="noopener noreferrer" title={r.snippet}>
                <span className="host">{hostOf(r.url)}</span>
                <span className="t">{r.title || r.url}</span>
              </a>
            ))}
          </div>
        )}
      </details>
    )
  }
  return (
    <div className="tool-chip">
      {p.status === 'running' ? <span className="spinner" /> : <FileText size={14} />}
      <span>Read</span>
      <a className="q" href={p.url} target="_blank" rel="noopener noreferrer">
        {p.title || hostOf(p.url)}
      </a>
      {p.status === 'error' && <span className="error-text">{p.error}</span>}
    </div>
  )
}

function ChatSettings({ initial, onClose, onSave }: { initial: string; onClose: () => void; onSave: (s: string) => void }) {
  const [s, setS] = useState(initial)
  return (
    <Dialog title="Chat instructions" onClose={onClose}>
      <label className="field">
        <span>Instructions for the model in this chat</span>
        <textarea className="input" rows={8} value={s} onChange={(e) => setS(e.target.value)} placeholder="e.g. Answer in German. Be concise. You are reviewing Rust code." />
        <div className="hint">Added to your organization's instructions for every answer in this chat.</div>
      </label>
      <div className="actions">
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" onClick={() => onSave(s)}>Save</button>
      </div>
    </Dialog>
  )
}

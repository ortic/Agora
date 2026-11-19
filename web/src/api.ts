// Typed client for the Agora REST API.

export class ApiError extends Error {
  status: number
  code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { 'x-agora': '1' }
  let payload: BodyInit | undefined
  if (body !== undefined) {
    headers['content-type'] = 'application/json'
    payload = JSON.stringify(body)
  }
  const res = await fetch(`/api${path}`, { method, headers, body: payload, credentials: 'same-origin' })
  if (res.status === 204) return undefined as T
  const text = await res.text()
  let data: any
  try {
    data = text ? JSON.parse(text) : undefined
  } catch {
    data = undefined
  }
  if (!res.ok) throw new ApiError(res.status, data?.error ?? 'error', data?.message ?? res.statusText)
  return data as T
}

export const api = {
  get: <T>(p: string) => request<T>('GET', p),
  post: <T>(p: string, b?: unknown) => request<T>('POST', p, b ?? {}),
  put: <T>(p: string, b?: unknown) => request<T>('PUT', p, b ?? {}),
  patch: <T>(p: string, b?: unknown) => request<T>('PATCH', p, b ?? {}),
  del: <T>(p: string) => request<T>('DELETE', p),
}

/** POSTs (or GETs) and reads a server-sent event stream of JSON events. */
export async function streamEvents(
  path: string,
  body: unknown | undefined,
  onEvent: (e: StreamEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const init: RequestInit = { method: body === undefined ? 'GET' : 'POST', headers: { 'x-agora': '1' }, credentials: 'same-origin', signal }
  if (body !== undefined) {
    ;(init.headers as Record<string, string>)['content-type'] = 'application/json'
    init.body = JSON.stringify(body)
  }
  const res = await fetch(`/api${path}`, init)
  if (!res.ok) {
    const data = await res.json().catch(() => undefined)
    throw new ApiError(res.status, data?.error ?? 'error', data?.message ?? res.statusText)
  }
  const reader = res.body!.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    buf += decoder.decode(value, { stream: true })
    let idx: number
    while ((idx = buf.indexOf('\n\n')) >= 0) {
      const raw = buf.slice(0, idx)
      buf = buf.slice(idx + 2)
      const data = raw
        .split('\n')
        .filter((l) => l.startsWith('data:'))
        .map((l) => l.slice(5).replace(/^ /, ''))
        .join('\n')
      if (!data) continue
      try {
        onEvent(JSON.parse(data))
      } catch {
        /* keep-alive or garbage */
      }
    }
  }
}

// ---- types (mirror the server's camelCase JSON) ----

export type Role = 'owner' | 'admin' | 'auditor' | 'member'

export interface User {
  id: string
  email: string | null
  name: string
  role: Role
  status: string
}

export interface Me {
  user: User | null
  organization: string
  notice: string | null
  webSearch: boolean
  defaultModelId: string | null
  allowUserSystemPrompt: boolean
  /** may sign in with Agora Desktop */
  desktop: boolean
}

export const canAdmin = (u?: User | null) => u?.role === 'owner' || u?.role === 'admin'
export const canAudit = (u?: User | null) => canAdmin(u) || u?.role === 'auditor'

export interface ModelInfo {
  id: string
  name: string
  model: string
  description: string | null
  provider: string
  kind: 'anthropic' | 'openai'
  contextWindow: number | null
  inputPrice: number
  outputPrice: number
  tools: boolean
  vision: boolean
  reasoning: boolean
  effort: string | null
  webSearch: boolean
}

export interface ChatSummary {
  id: string
  title: string
  pinned: boolean
  modelId: string | null
  updatedAt: string
}

export interface Chat {
  id: string
  userId: string
  title: string
  modelId: string | null
  systemPrompt: string | null
  settings: { webSearch?: boolean; knowledge?: boolean; effort?: string }
  leafId: string | null
  pinned: boolean
  createdAt: string
  updatedAt: string
  deletedAt: string | null
}

export interface SearchHit {
  title: string
  url: string
  snippet?: string
}

export type Part =
  | { type: 'reasoning'; text: string }
  | { type: 'search'; id: string; engine: string; query: string; status: 'running' | 'done' | 'error'; results?: SearchHit[]; error?: string | null }
  | { type: 'fetch'; id: string; url: string; title?: string; status: 'running' | 'done' | 'error'; error?: string | null }
  | { type: 'knowledge'; id: string; source: string; query: string; status: 'running' | 'done' | 'error'; results?: SearchHit[]; error?: string | null }
  | ToolPart
  | { type: 'attachment'; id: string; name: string; mediaType: string; size: number }
  | { type: 'agent'; id: string; description: string; prompt: string; status: 'running' | 'done' | 'error' | 'stopped'; steps: number; output?: string; error?: string | null }
  | { type: 'compaction'; id: string; status: 'running' | 'done' | 'error'; error?: string | null }

/** A tool a model ran on the person's computer through Agora Desktop. */
export interface ToolPart {
  type: 'tool'
  id: string
  /** the sub-agent that made the call */
  agent?: string | null
  feature: 'code' | 'browser' | 'connector'
  connector: string | null
  tool: string
  input: Record<string, unknown> | null
  status: 'running' | 'done' | 'error' | 'denied' | 'blocked' | 'timeout'
  output?: string
  error?: string | null
  /** images and PDFs the tool showed the model */
  media?: { name: string; mediaType: string }[]
}

/** An image or a PDF sent with a message (base64). */
export interface AttachmentIn {
  name: string
  mediaType: string
  data: string
  size: number
}

export interface Device {
  id: string
  userId: string
  user: string | null
  name: string
  platform: string | null
  appVersion: string | null
  createdAt: string
  lastSeenAt: string | null
  revokedAt: string | null
  connected: boolean
}

export interface Message {
  id: string
  chatId: string
  parentId: string | null
  role: 'user' | 'assistant'
  content: string
  parts: Part[]
  modelId: string | null
  modelName: string | null
  status: 'complete' | 'streaming' | 'error' | 'stopped'
  error: string | null
  inputTokens: number
  outputTokens: number
  costUsd: number
  durationMs: number | null
  createdAt: string
}

export type StreamEvent =
  | { type: 'start'; userMessage: Message; assistantMessage: Message }
  | { type: 'snapshot'; content: string; parts: Part[] }
  | { type: 'text'; d: string }
  | { type: 'reasoning'; d: string }
  | { type: 'part'; part: Part }
  | { type: 'done'; message: Message }
  | { type: 'title'; title: string }
  | { type: 'end' }

export interface ConnectorInfo {
  id: string
  name: string
  kind: 'wikicraft'
  baseUrl: string
  description: string | null
  connected: boolean
  tokenHint: string | null
  lastUsedAt: string | null
}

export interface Budget {
  limit: number | null
  source: 'user' | 'group' | 'default' | 'none'
  spent: number
}

// ---- formatting helpers ----

export function usd(n: number | null | undefined, digits?: number): string {
  if (n == null) return '—'
  const d = digits ?? (Math.abs(n) >= 100 ? 0 : Math.abs(n) >= 1 ? 2 : Math.abs(n) >= 0.01 ? 3 : 4)
  return `$${n.toLocaleString('en-US', { minimumFractionDigits: Math.min(d, 2), maximumFractionDigits: d })}`
}

/** `1 passage`, `3 passages` */
export function plural(n: number, one: string, many: string = `${one}s`): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`
}

export function num(n: number | null | undefined): string {
  if (n == null) return '—'
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e4) return `${(n / 1e3).toFixed(1)}k`
  return n.toLocaleString('en-US')
}

export function when(iso: string | null | undefined): string {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
}

export function ago(iso: string | null | undefined): string {
  if (!iso) return 'never'
  const s = (Date.now() - new Date(iso).getTime()) / 1000
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`
  if (s < 86400 * 30) return `${Math.floor(s / 86400)} d ago`
  return new Date(iso).toLocaleDateString()
}

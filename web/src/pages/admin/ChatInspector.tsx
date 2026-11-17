import { useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api, num, usd, when, type Chat, type Message, type User } from '../../api'
import { Markdown } from '../../ui'

interface Inspect {
  chat: Chat
  owner: User | null
  messages: Message[]
  web: { at: string; kind: string; query: string | null; url: string | null; status: string }[]
}

export function ChatInspector() {
  const { id } = useParams()
  const d = useQuery({ queryKey: ['inspect', id], queryFn: () => api.get<Inspect>(`/admin/chats/${id}`), staleTime: Infinity })
  if (d.isLoading) return <div className="muted">Loading…</div>
  if (!d.data) return <div className="banner error">Conversation not found.</div>
  const { chat, owner, messages, web } = d.data
  return (
    <div style={{ maxWidth: 860 }}>
      <div className="banner" style={{ marginBottom: 16 }}>You're viewing someone else's conversation. This access was recorded in the audit log.</div>
      <h2 style={{ marginTop: 0 }}>{chat.title}</h2>
      <div className="muted" style={{ marginBottom: 16 }}>
        {owner?.name} ({owner?.email}) · started {when(chat.createdAt)}
        {chat.deletedAt && <span className="badge danger" style={{ marginLeft: 6 }}>deleted by the owner {when(chat.deletedAt)}</span>}
      </div>
      {chat.systemPrompt && <div className="card" style={{ marginBottom: 14 }}><div className="faint" style={{ fontSize: 12 }}>Chat instructions</div>{chat.systemPrompt}</div>}
      <p className="hint">All branches in the order they were written (edits and regenerated answers included).</p>
      {messages.map((m) => (
        <div key={m.id} className="card" style={{ marginBottom: 10, background: m.role === 'user' ? 'var(--bg-soft)' : undefined }}>
          <div className="row faint" style={{ fontSize: 12, marginBottom: 6 }}>
            <strong>{m.role === 'user' ? owner?.name ?? 'User' : m.modelName}</strong>
            <span>{when(m.createdAt)}</span>
            {m.role === 'assistant' && <span>{num(m.inputTokens)} in · {num(m.outputTokens)} out · {usd(m.costUsd)}</span>}
            {m.status !== 'complete' && <span className="badge warn">{m.status}</span>}
          </div>
          {m.role === 'user' ? <div style={{ whiteSpace: 'pre-wrap' }}>{m.content}</div> : <Markdown text={m.content || '*(no text)*'} />}
          {m.error && <div className="error-text">{m.error}</div>}
        </div>
      ))}
      {web.length > 0 && (
        <>
          <h3>Web activity in this conversation</h3>
          <ul>
            {web.map((w, i) => (
              <li key={i}>{when(w.at)} · {w.kind === 'search' ? `searched “${w.query}”` : `read ${w.url}`} · {w.status}</li>
            ))}
          </ul>
        </>
      )}
    </div>
  )
}

import { useMemo, useState, type ReactNode } from 'react'
import { Link, NavLink, useLocation, useNavigate, useParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { CircleHelp, LogOut, Menu, MessageSquarePlus, Pencil, Pin, PinOff, Search, Settings, Shield, Trash2 } from 'lucide-react'
import { api, canAudit, type ChatSummary, type Me } from '../api'
import { Avatar } from '../ui'

function groupOf(iso: string, pinned: boolean): string {
  if (pinned) return 'Pinned'
  const d = new Date(iso)
  const today = new Date()
  today.setHours(0, 0, 0, 0)
  const days = (today.getTime() - new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()) / 86400000
  if (days <= 0) return 'Today'
  if (days <= 1) return 'Yesterday'
  if (days <= 7) return 'Previous 7 days'
  if (days <= 30) return 'Previous 30 days'
  return d.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })
}

export function Layout({ me, children }: { me: Me; children: ReactNode }) {
  const [q, setQ] = useState('')
  const [navOpen, setNavOpen] = useState(false)
  const location = useLocation()
  const qc = useQueryClient()
  const chats = useQuery({
    queryKey: ['chats', q],
    queryFn: () => api.get<ChatSummary[]>(`/chats${q.trim() ? `?q=${encodeURIComponent(q.trim())}` : ''}`),
    placeholderData: (prev) => prev,
  })
  const groups = useMemo(() => {
    const out: [string, ChatSummary[]][] = []
    for (const c of chats.data ?? []) {
      const g = groupOf(c.updatedAt, c.pinned)
      const last = out[out.length - 1]
      if (last && last[0] === g) last[1].push(c)
      else out.push([g, [c]])
    }
    return out
  }, [chats.data])
  const logout = useMutation({
    mutationFn: () => api.post('/auth/logout'),
    onSuccess: () => {
      qc.clear()
      window.location.href = '/'
    },
  })
  const user = me.user!
  return (
    <div className={`shell${navOpen ? ' nav-open' : ''}`} onClick={(e) => navOpen && (e.target as HTMLElement).closest('a') && setNavOpen(false)}>
      <aside className="sidebar" aria-label="Chats">
        <div className="sidebar-top">
          <Link to="/" className="brand" style={{ color: 'inherit', textDecoration: 'none' }}>
            <img src="/favicon.svg" alt="" />
            <span>{me.organization}</span>
          </Link>
          <Link to="/" className="btn" style={{ justifyContent: 'flex-start' }}>
            <MessageSquarePlus size={16} /> New chat
          </Link>
          <div className="sidebar-search">
            <Search size={15} />
            <input className="input" placeholder="Search chats" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search chats" />
          </div>
        </div>
        <nav className="chat-list">
          {groups.map(([g, items]) => (
            <div key={g}>
              <div className="chat-group">{g}</div>
              {items.map((c) => (
                <ChatItem key={c.id} chat={c} />
              ))}
            </div>
          ))}
          {chats.data?.length === 0 && <div className="faint" style={{ padding: '16px 8px', fontSize: 13 }}>{q ? 'No chats found.' : 'Your chats show up here.'}</div>}
        </nav>
        <div className="sidebar-foot">
          {canAudit(user) && (
            <NavLink to="/admin" className={({ isActive }) => `nav-link${isActive ? ' active' : ''}`}>
              <Shield size={16} /> Administration
            </NavLink>
          )}
          <NavLink to="/settings" className={({ isActive }) => `nav-link${isActive ? ' active' : ''}`}>
            <Settings size={16} /> Settings & usage
          </NavLink>
          <NavLink to="/help" className={({ isActive }) => `nav-link${isActive ? ' active' : ''}`}>
            <CircleHelp size={16} /> Help
          </NavLink>
          <div className="user-chip">
            <Avatar name={user.name} />
            <div className="grow" style={{ minWidth: 0 }}>
              <div style={{ fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{user.name}</div>
              <div className="faint" style={{ fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{user.email}</div>
            </div>
            <button className="icon-btn" title="Sign out" aria-label="Sign out" onClick={() => logout.mutate()}>
              <LogOut size={16} />
            </button>
          </div>
        </div>
      </aside>
      <main className="main">
        <div className="mobile-bar">
          <button className="icon-btn" aria-label="Open the chat list" onClick={() => setNavOpen(true)}>
            <Menu size={18} />
          </button>
          <span style={{ fontWeight: 600 }}>{me.organization}</span>
        </div>
        {children}
      </main>
      {navOpen && <div style={{ position: 'fixed', inset: 0, zIndex: 35 }} onClick={() => setNavOpen(false)} key={location.pathname} />}
    </div>
  )
}

function ChatItem({ chat }: { chat: ChatSummary }) {
  const { id } = useParams()
  const active = location.pathname === `/c/${chat.id}` || id === chat.id
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState(chat.title)
  const qc = useQueryClient()
  const navigate = useNavigate()
  const update = useMutation({
    mutationFn: (b: { title?: string; pinned?: boolean }) => api.patch(`/chats/${chat.id}`, b),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['chats'] }),
  })
  const remove = useMutation({
    mutationFn: () => api.del(`/chats/${chat.id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chats'] })
      if (active) navigate('/')
    },
  })
  if (editing) {
    return (
      <div className="chat-item active">
        <input
          autoFocus
          value={title}
          aria-label="Chat title"
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => {
            setEditing(false)
            if (title.trim() && title !== chat.title) update.mutate({ title })
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
            if (e.key === 'Escape') {
              setTitle(chat.title)
              setEditing(false)
            }
          }}
        />
      </div>
    )
  }
  return (
    <Link to={`/c/${chat.id}`} className={`chat-item${active ? ' active' : ''}`} aria-current={active ? 'page' : undefined}>
      <span className="title">{chat.title}</span>
      <span className="actions" onClick={(e) => e.preventDefault()}>
        <button className="icon-btn" title={chat.pinned ? 'Unpin' : 'Pin'} aria-label={chat.pinned ? 'Unpin' : 'Pin'} onClick={() => update.mutate({ pinned: !chat.pinned })}>
          {chat.pinned ? <PinOff size={14} /> : <Pin size={14} />}
        </button>
        <button className="icon-btn" title="Rename" aria-label="Rename" onClick={() => setEditing(true)}>
          <Pencil size={14} />
        </button>
        <button
          className="icon-btn"
          title="Delete"
          aria-label="Delete"
          onClick={() => {
            if (confirm(`Delete “${chat.title}”?`)) remove.mutate()
          }}
        >
          <Trash2 size={14} />
        </button>
      </span>
    </Link>
  )
}

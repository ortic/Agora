import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ago, api, canAdmin, num, usd, type Me, type Role } from '../../api'
import { Dialog, ErrorText } from '../../ui'

export interface AdminUser {
  id: string
  email: string | null
  name: string
  role: Role
  status: 'active' | 'deactivated'
  monthlyBudgetUsd: number | null
  lastLoginAt: string | null
  lastActiveAt: string | null
  spentThisMonth: number
  requestsThisMonth: number
  groups: string[]
  localPassword: boolean
}

interface UserChat {
  id: string
  title: string
  createdAt: string
  updatedAt: string
  deletedAt: string | null
  messages: number
  costUsd: number
}

export function UsersAdmin({ me }: { me: Me }) {
  const [q, setQ] = useState('')
  const [editing, setEditing] = useState<AdminUser | 'new' | null>(null)
  const [viewing, setViewing] = useState<AdminUser | null>(null)
  const users = useQuery({ queryKey: ['admin-users', q], queryFn: () => api.get<AdminUser[]>(`/admin/users${q ? `?q=${encodeURIComponent(q)}` : ''}`), placeholderData: (p) => p })
  const admin = canAdmin(me.user)
  return (
    <>
      <div className="row" style={{ marginBottom: 12 }}>
        <input className="input" style={{ maxWidth: 320 }} placeholder="Search name or e-mail" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search users" />
        <div className="grow" />
        {admin && <button className="btn primary" onClick={() => setEditing('new')}>Add person</button>}
      </div>
      <div className="table-wrap">
        <table className="data">
          <thead>
            <tr>
              <th>Name</th>
              <th>Role</th>
              <th>Groups</th>
              <th className="num">Requests (month)</th>
              <th className="num">Spent (month)</th>
              <th className="num">Budget</th>
              <th>Last active</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {users.data?.map((u) => (
              <tr key={u.id}>
                <td>
                  <div style={{ fontWeight: 600 }}>{u.name}</div>
                  <div className="faint" style={{ fontSize: 12.5 }}>{u.email}</div>
                </td>
                <td>
                  <span className={`badge${u.role === 'owner' || u.role === 'admin' ? ' accent' : u.role === 'auditor' ? ' warn' : ''}`}>{u.role}</span>
                  {u.status === 'deactivated' && <span className="badge danger" style={{ marginLeft: 4 }}>deactivated</span>}
                </td>
                <td className="muted">{u.groups.join(', ') || '—'}</td>
                <td className="num">{num(u.requestsThisMonth)}</td>
                <td className="num">{usd(u.spentThisMonth)}</td>
                <td className="num">{u.monthlyBudgetUsd != null ? usd(u.monthlyBudgetUsd, 2) : <span className="faint">inherited</span>}</td>
                <td>{ago(u.lastActiveAt)}</td>
                <td style={{ whiteSpace: 'nowrap', textAlign: 'right' }}>
                  <button className="btn small ghost" onClick={() => setViewing(u)}>Chats</button>
                  {admin && <button className="btn small ghost" onClick={() => setEditing(u)}>Edit</button>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {editing && <UserDialog user={editing === 'new' ? null : editing} me={me} onClose={() => setEditing(null)} />}
      {viewing && <UserChats user={viewing} onClose={() => setViewing(null)} />}
    </>
  )
}

function UserDialog({ user, me, onClose }: { user: AdminUser | null; me: Me; onClose: () => void }) {
  const qc = useQueryClient()
  const [form, setForm] = useState({
    name: user?.name ?? '',
    email: user?.email ?? '',
    role: user?.role ?? 'member',
    status: user?.status ?? 'active',
    budget: user?.monthlyBudgetUsd != null ? String(user.monthlyBudgetUsd) : '',
    password: '',
  })
  const save = useMutation({
    mutationFn: () => {
      const budget = form.budget.trim() === '' ? null : Number(form.budget)
      if (!user) return api.post('/admin/users', { name: form.name, email: form.email, role: form.role, password: form.password || null })
      return api.patch(`/admin/users/${user.id}`, {
        name: form.name,
        role: form.role,
        status: form.status,
        monthlyBudgetUsd: budget,
        password: form.password || undefined,
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-users'] })
      onClose()
    },
  })
  return (
    <Dialog title={user ? `Edit ${user.name}` : 'Add a person'} onClose={onClose}>
      <label className="field">
        <span>Name</span>
        <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
      </label>
      {!user && (
        <label className="field">
          <span>E-mail</span>
          <input className="input" type="email" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
          <div className="hint">With single sign-on, the account is linked when they first sign in with this address.</div>
        </label>
      )}
      <div className="two-col">
        <label className="field">
          <span>Role</span>
          <select className="input" value={form.role} onChange={(e) => setForm({ ...form, role: e.target.value as Role })}>
            <option value="member">Member: chats</option>
            <option value="auditor">Auditor: reads audit log, usage, conversations</option>
            <option value="admin">Admin: everything but owners</option>
            {me.user!.role === 'owner' && <option value="owner">Owner</option>}
          </select>
        </label>
        {user && (
          <label className="field">
            <span>Status</span>
            <select className="input" value={form.status} onChange={(e) => setForm({ ...form, status: e.target.value as 'active' })}>
              <option value="active">Active</option>
              <option value="deactivated">Deactivated (signed out, can't sign in)</option>
            </select>
          </label>
        )}
      </div>
      {user && (
        <label className="field">
          <span>Monthly budget (USD)</span>
          <input className="input" inputMode="decimal" value={form.budget} onChange={(e) => setForm({ ...form, budget: e.target.value })} placeholder="inherit from groups / default" />
          <div className="hint">Overrides group and default budgets. Empty: inherit.</div>
        </label>
      )}
      <label className="field">
        <span>{user ? 'New password' : 'Password'}</span>
        <input className="input" type="password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} autoComplete="new-password" placeholder={user ? 'unchanged' : 'none: single sign-on only'} />
      </label>
      <ErrorText error={save.error} />
      <div className="actions">
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={save.isPending || !form.name.trim()} onClick={() => save.mutate()}>Save</button>
      </div>
    </Dialog>
  )
}

function UserChats({ user, onClose }: { user: AdminUser; onClose: () => void }) {
  const chats = useQuery({ queryKey: ['user-chats', user.id], queryFn: () => api.get<UserChat[]>(`/admin/users/${user.id}/chats`) })
  return (
    <Dialog title={`Conversations of ${user.name}`} onClose={onClose} wide>
      <p className="muted" style={{ marginTop: 0 }}>Opening a conversation is recorded in the audit log.</p>
      <div className="table-wrap">
        <table className="data">
          <thead>
            <tr>
              <th>Title</th>
              <th className="num">Messages</th>
              <th className="num">Cost</th>
              <th>Updated</th>
            </tr>
          </thead>
          <tbody>
            {chats.data?.map((c) => (
              <tr key={c.id}>
                <td>
                  <Link to={`/admin/chats/${c.id}`}>{c.title}</Link>
                  {c.deletedAt && <span className="badge danger" style={{ marginLeft: 6 }}>deleted</span>}
                </td>
                <td className="num">{c.messages}</td>
                <td className="num">{usd(c.costUsd)}</td>
                <td>{ago(c.updatedAt)}</td>
              </tr>
            ))}
            {chats.data?.length === 0 && (
              <tr>
                <td colSpan={4} className="empty">No conversations.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </Dialog>
  )
}

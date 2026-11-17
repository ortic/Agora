import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, canAdmin, usd, type Me } from '../../api'
import { Dialog, ErrorText } from '../../ui'
import type { AdminUser } from './Users'

export interface Group {
  id: string
  name: string
  description: string | null
  managedBy: string | null
  monthlyBudgetUsd: number | null
  memberIds: string[]
  spentThisMonth: number
}

export function GroupsAdmin({ me }: { me: Me }) {
  const groups = useQuery({ queryKey: ['admin-groups'], queryFn: () => api.get<Group[]>('/admin/groups') })
  const [editing, setEditing] = useState<Group | 'new' | null>(null)
  const admin = canAdmin(me.user)
  return (
    <>
      <div className="row" style={{ marginBottom: 12 }}>
        <p className="muted grow" style={{ margin: 0 }}>Groups decide who may use which models and carry budgets. Groups from your identity provider are kept in sync at sign-in.</p>
        {admin && <button className="btn primary" onClick={() => setEditing('new')}>New group</button>}
      </div>
      <div className="table-wrap">
        <table className="data">
          <thead>
            <tr>
              <th>Group</th>
              <th className="num">Members</th>
              <th className="num">Budget per person</th>
              <th className="num">Spent (month)</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {groups.data?.map((g) => (
              <tr key={g.id}>
                <td>
                  <div style={{ fontWeight: 600 }}>{g.name}</div>
                  <div className="faint" style={{ fontSize: 12.5 }}>
                    {g.description}
                    {g.managedBy && <span className="badge" style={{ marginLeft: 4 }}>synced from {g.managedBy}</span>}
                  </div>
                </td>
                <td className="num">{g.memberIds.length}</td>
                <td className="num">{g.monthlyBudgetUsd != null ? usd(g.monthlyBudgetUsd, 2) : '—'}</td>
                <td className="num">{usd(g.spentThisMonth)}</td>
                <td style={{ textAlign: 'right' }}>{admin && <button className="btn small ghost" onClick={() => setEditing(g)}>Edit</button>}</td>
              </tr>
            ))}
            {groups.data?.length === 0 && (
              <tr>
                <td colSpan={5} className="empty">No groups yet.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {editing && <GroupDialog group={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </>
  )
}

function GroupDialog({ group, onClose }: { group: Group | null; onClose: () => void }) {
  const qc = useQueryClient()
  const users = useQuery({ queryKey: ['admin-users', ''], queryFn: () => api.get<AdminUser[]>('/admin/users') })
  const [name, setName] = useState(group?.name ?? '')
  const [description, setDescription] = useState(group?.description ?? '')
  const [budget, setBudget] = useState(group?.monthlyBudgetUsd != null ? String(group.monthlyBudgetUsd) : '')
  const [members, setMembers] = useState<Set<string>>(new Set(group?.memberIds ?? []))
  const [filter, setFilter] = useState('')
  const save = useMutation({
    mutationFn: () => {
      const body = { name, description, monthlyBudgetUsd: budget.trim() ? Number(budget) : null, memberIds: [...members] }
      return group ? api.patch(`/admin/groups/${group.id}`, body) : api.post('/admin/groups', body)
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-groups'] })
      qc.invalidateQueries({ queryKey: ['admin-users'] })
      onClose()
    },
  })
  const remove = useMutation({
    mutationFn: () => api.del(`/admin/groups/${group!.id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['admin-groups'] })
      onClose()
    },
  })
  const shown = (users.data ?? []).filter((u) => !filter || `${u.name} ${u.email}`.toLowerCase().includes(filter.toLowerCase()))
  return (
    <Dialog title={group ? `Edit ${group.name}` : 'New group'} onClose={onClose}>
      <label className="field">
        <span>Name</span>
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label className="field">
        <span>Description</span>
        <input className="input" value={description} onChange={(e) => setDescription(e.target.value)} />
      </label>
      <label className="field">
        <span>Monthly budget per member (USD)</span>
        <input className="input" inputMode="decimal" value={budget} onChange={(e) => setBudget(e.target.value)} placeholder="none" />
        <div className="hint">People in several groups get the highest of their groups' budgets; a personal budget overrides it.</div>
      </label>
      <div className="field">
        <span style={{ fontWeight: 500 }}>Members ({members.size})</span>
        {group?.managedBy && <div className="hint">This group is synced from {group.managedBy}: changes are replaced at members' next sign-in.</div>}
        <input className="input" style={{ margin: '6px 0' }} placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <div style={{ maxHeight: 220, overflowY: 'auto', border: '1px solid var(--border)', borderRadius: 8, padding: '6px 10px' }}>
          {shown.map((u) => (
            <label key={u.id} className="check" style={{ marginBottom: 4 }}>
              <input
                type="checkbox"
                checked={members.has(u.id)}
                onChange={(e) => {
                  const s = new Set(members)
                  if (e.target.checked) s.add(u.id)
                  else s.delete(u.id)
                  setMembers(s)
                }}
              />
              {u.name} <span className="faint">{u.email}</span>
            </label>
          ))}
        </div>
      </div>
      <ErrorText error={save.error ?? remove.error} />
      <div className="actions">
        {group && (
          <button className="btn danger" style={{ marginRight: 'auto' }} onClick={() => confirm(`Delete the group ${group.name}?`) && remove.mutate()}>
            Delete
          </button>
        )}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!name.trim() || save.isPending} onClick={() => save.mutate()}>Save</button>
      </div>
    </Dialog>
  )
}

import { NavLink, Route, Routes } from 'react-router-dom'
import { BarChart3, BookOpen, Bot, FileClock, Globe, LayoutDashboard, MonitorSmartphone, Settings2, Users, UsersRound, Wrench } from 'lucide-react'
import { canAdmin, type Me } from '../api'
import { Overview } from './admin/Overview'
import { UsersAdmin } from './admin/Users'
import { GroupsAdmin } from './admin/Groups'
import { ModelsAdmin } from './admin/Models'
import { SettingsAdmin } from './admin/Settings'
import { UsageAdmin } from './admin/Usage'
import { AuditAdmin } from './admin/Audit'
import { WebAdmin } from './admin/Web'
import { DesktopAdmin } from './admin/Desktop'
import { McpAdmin } from './admin/Mcp'
import { ChatInspector } from './admin/ChatInspector'

export function AdminPage({ me }: { me: Me }) {
  const admin = canAdmin(me.user)
  const tabs = [
    { to: '/admin', label: 'Overview', icon: LayoutDashboard, end: true, show: true },
    { to: '/admin/users', label: 'Users', icon: Users, show: true },
    { to: '/admin/groups', label: 'Groups', icon: UsersRound, show: true },
    { to: '/admin/models', label: 'Models', icon: Bot, show: admin },
    { to: '/admin/usage', label: 'Usage', icon: BarChart3, show: true },
    { to: '/admin/audit', label: 'Audit log', icon: FileClock, show: true },
    { to: '/admin/web', label: 'Web activity', icon: Globe, show: true },
    { to: '/admin/mcp', label: 'MCP servers', icon: Wrench, show: true },
    { to: '/admin/desktop', label: 'Desktop', icon: MonitorSmartphone, show: true },
    { to: '/admin/settings', label: 'Settings', icon: Settings2, show: admin },
    { to: '/help/models', label: 'Help', icon: BookOpen, show: true },
  ]
  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-head">
          <div>
            <h1>Administration</h1>
            <div className="muted">{admin ? 'Models, people, budgets and oversight' : 'Oversight (read-only, auditor)'}</div>
          </div>
        </div>
        <nav className="tabs">
          {tabs
            .filter((t) => t.show)
            .map((t) => (
              <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `tab${isActive ? ' active' : ''}`}>
                <t.icon size={15} /> {t.label}
              </NavLink>
            ))}
        </nav>
        <Routes>
          <Route index element={<Overview />} />
          <Route path="users" element={<UsersAdmin me={me} />} />
          <Route path="groups" element={<GroupsAdmin me={me} />} />
          {admin && <Route path="models" element={<ModelsAdmin />} />}
          <Route path="usage" element={<UsageAdmin />} />
          <Route path="audit" element={<AuditAdmin />} />
          <Route path="web" element={<WebAdmin />} />
          <Route path="mcp" element={<McpAdmin admin={admin} />} />
          <Route path="desktop" element={<DesktopAdmin admin={admin} />} />
          <Route path="chats/:id" element={<ChatInspector />} />
          {admin && <Route path="settings" element={<SettingsAdmin />} />}
        </Routes>
      </div>
    </div>
  )
}

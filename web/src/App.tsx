import { useQuery } from '@tanstack/react-query'
import { Navigate, Route, Routes } from 'react-router-dom'
import { api, canAudit, type Me } from './api'
import { Layout } from './components/Layout'
import { Login, Setup } from './pages/Auth'
import { ChatPage } from './pages/Chat'
import { SettingsPage } from './pages/Settings'
import { AdminPage } from './pages/Admin'
import { DesktopLink } from './pages/DesktopLink'
import { HelpPage } from './pages/Help'

export function useMe() {
  return useQuery({ queryKey: ['me'], queryFn: () => api.get<Me>('/me'), staleTime: 60_000 })
}

export function App() {
  const me = useMe()
  const setup = useQuery({ queryKey: ['setup'], queryFn: () => api.get<{ needsSetup: boolean }>('/setup'), enabled: me.isSuccess && !me.data.user })
  if (me.isLoading) return null
  if (me.isError) return <div className="auth"><div className="banner error">Agora can't be reached: {String(me.error.message)}</div></div>
  const user = me.data!.user
  if (!user) {
    if (setup.isLoading) return null
    if (setup.data?.needsSetup) return <Setup />
    return <Login organization={me.data!.organization} />
  }
  return (
    <Layout me={me.data!}>
      <Routes>
        <Route path="/" element={<ChatPage key="new" me={me.data!} />} />
        <Route path="/c/:id" element={<ChatPage me={me.data!} />} />
        <Route path="/desktop/link" element={<DesktopLink />} />
        <Route path="/settings/*" element={<SettingsPage me={me.data!} />} />
        <Route path="/help/*" element={<HelpPage me={me.data!} />} />
        <Route path="/admin/*" element={canAudit(user) ? <AdminPage me={me.data!} /> : <Navigate to="/" replace />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </Layout>
  )
}

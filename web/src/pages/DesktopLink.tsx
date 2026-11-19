import { useState, type FormEvent } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useMutation, useQuery } from '@tanstack/react-query'
import { MonitorSmartphone } from 'lucide-react'
import { api, ApiError, when } from '../api'
import { ErrorText } from '../ui'

interface LoginInfo {
  deviceName: string
  platform: string | null
  appVersion: string | null
  ip: string | null
  status: 'pending' | 'approved' | 'denied'
  createdAt: string
  allowed: boolean
}

/** `/desktop/link?code=XXXX-XXXX`: confirm that Agora Desktop on a computer may sign in as you. */
export function DesktopLink() {
  const [params, setParams] = useSearchParams()
  const code = (params.get('code') ?? '').trim().toUpperCase()
  const [typed, setTyped] = useState('')
  const [done, setDone] = useState<'approved' | 'denied' | null>(null)
  const info = useQuery({
    queryKey: ['desktop-login', code],
    queryFn: () => api.get<LoginInfo>(`/desktop/login/${encodeURIComponent(code)}`),
    enabled: !!code,
    retry: false,
    staleTime: Infinity,
  })
  const decide = useMutation({
    mutationFn: (approve: boolean) => api.post(`/desktop/login/${encodeURIComponent(code)}`, { approve }),
    onSuccess: (_, approve) => setDone(approve ? 'approved' : 'denied'),
  })
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (typed.trim()) setParams({ code: typed.trim().toUpperCase() })
  }
  const d = info.data
  const state = done ?? (d && d.status !== 'pending' ? d.status : null)
  const notFound = info.error instanceof ApiError && info.error.status === 404
  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 520 }}>
        <div className="page-head">
          <div>
            <h1><MonitorSmartphone size={20} style={{ verticalAlign: -3 }} /> Connect Agora Desktop</h1>
            <div className="muted">Sign the app on a computer in with your account</div>
          </div>
        </div>
        {!code || notFound ? (
          <form className="card" onSubmit={submit}>
            {notFound && <div className="banner error" style={{ marginBottom: 12 }}>This code isn't valid (any more). Codes last ten minutes: start signing in again in the app.</div>}
            <label className="field">
              <span>Code shown in Agora Desktop</span>
              <input className="input mono" value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="ABCD-EFGH" autoFocus autoCapitalize="characters" autoComplete="off" />
            </label>
            <div className="actions"><button className="btn primary" disabled={!typed.trim()}>Continue</button></div>
          </form>
        ) : info.isLoading ? (
          <div className="muted">Loading…</div>
        ) : !d ? (
          <ErrorText error={info.error} />
        ) : state === 'approved' ? (
          <div className="banner ok"><strong>{d.deviceName} is connected.</strong> You can go back to the app; this page can be closed. Devices are listed, and signed out, under Settings.</div>
        ) : state === 'denied' ? (
          <div className="banner">The sign-in was refused. Nothing was connected.</div>
        ) : (
          <div className="card">
            <div style={{ fontSize: 26, letterSpacing: 2, textAlign: 'center', margin: '4px 0 14px' }} className="mono"><strong>{code}</strong></div>
            <table className="data" style={{ marginBottom: 14 }}>
              <tbody>
                <tr><td className="faint">Computer</td><td><strong>{d.deviceName}</strong></td></tr>
                {d.platform && <tr><td className="faint">System</td><td>{d.platform}</td></tr>}
                {d.appVersion && <tr><td className="faint">App version</td><td>{d.appVersion}</td></tr>}
                {d.ip && <tr><td className="faint">Address</td><td className="mono">{d.ip}</td></tr>}
                <tr><td className="faint">Started</td><td>{when(d.createdAt)}</td></tr>
              </tbody>
            </table>
            {d.allowed ? (
              <>
                <div className="banner" style={{ marginBottom: 14 }}>
                  Only continue if you just started Agora Desktop on this computer and it shows this code. The app will act as you: it uses your models and
                  budget, and models can work on that computer with what your organization allows.
                </div>
                <ErrorText error={decide.error} />
                <div className="actions" style={{ display: 'flex', gap: 8 }}>
                  <button className="btn" disabled={decide.isPending} onClick={() => decide.mutate(false)}>Deny</button>
                  <button className="btn primary" disabled={decide.isPending} onClick={() => decide.mutate(true)}>Approve</button>
                </div>
              </>
            ) : (
              <>
                <div className="banner error" style={{ marginBottom: 14 }}>Agora Desktop isn't enabled for your account. Ask an administrator.</div>
                <ErrorText error={decide.error} />
                <div className="actions"><button className="btn" disabled={decide.isPending} onClick={() => decide.mutate(false)}>Refuse the sign-in</button></div>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

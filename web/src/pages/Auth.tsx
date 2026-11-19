import { useState, type FormEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '../api'
import { ErrorText } from '../ui'

interface Providers {
  passwordLogin: boolean
  providers: { id: string; name: string; type: string; loginUrl: string | null }[]
}

export function Login({ organization }: { organization: string }) {
  const params = new URLSearchParams(window.location.search)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(params.get('error'))
  const [busy, setBusy] = useState(false)
  const providers = useQuery({ queryKey: ['providers'], queryFn: () => api.get<Providers>('/auth/providers') })
  const redirects = providers.data?.providers.filter((p) => p.loginUrl) ?? []
  const hasLdap = providers.data?.providers.some((p) => p.type === 'ldap')
  const showForm = providers.data ? providers.data.passwordLogin || hasLdap : true
  // the page asked for, with its query (e.g. /desktop/link?code=…), so it survives signing in
  const next = params.get('next') ?? (window.location.pathname === '/' ? '/' : window.location.pathname + window.location.search)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.post('/auth/login', { email, password })
      window.location.href = next.startsWith('/') && !next.startsWith('//') ? next : '/'
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setBusy(false)
    }
  }

  return (
    <div className="auth">
      <div className="auth-card">
        <div className="brand">
          <img src="/favicon.svg" alt="" />
          <span>{organization}</span>
        </div>
        <p className="muted" style={{ textAlign: 'center', marginTop: 0 }}>Sign in to continue</p>
        {redirects.map((p) => (
          <a key={p.id} className="btn" style={{ width: '100%', justifyContent: 'center', marginBottom: 8 }} href={`${p.loginUrl}?next=${encodeURIComponent(next)}`}>
            Continue with {p.name}
          </a>
        ))}
        {redirects.length > 0 && showForm && <div className="divider">or</div>}
        {showForm && (
          <form onSubmit={submit}>
            <label className="field">
              <span>{hasLdap ? 'E-mail or username' : 'E-mail'}</span>
              <input className="input" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
            </label>
            <label className="field">
              <span>Password</span>
              <input className="input" type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
            </label>
            <button className="btn primary" style={{ width: '100%', justifyContent: 'center' }} disabled={busy}>
              {busy ? 'Signing in…' : 'Sign in'}
            </button>
          </form>
        )}
        {error && <ErrorText error={error} />}
      </div>
    </div>
  )
}

export function Setup() {
  const [form, setForm] = useState({ organization: '', name: '', email: '', password: '' })
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, [k]: e.target.value })
  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.post('/setup', form)
      window.location.href = '/admin/models'
    } catch (err) {
      setError(err)
      setBusy(false)
    }
  }
  return (
    <div className="auth">
      <form className="auth-card" onSubmit={submit}>
        <div className="brand">
          <img src="/favicon.svg" alt="" />
          <span>Welcome to Agora</span>
        </div>
        <p className="muted" style={{ textAlign: 'center', marginTop: 0 }}>Create the owner account. You'll connect models next.</p>
        <label className="field">
          <span>Organization</span>
          <input className="input" value={form.organization} onChange={set('organization')} placeholder="Acme Corp" />
        </label>
        <label className="field">
          <span>Your name</span>
          <input className="input" value={form.name} onChange={set('name')} required autoComplete="name" />
        </label>
        <label className="field">
          <span>E-mail</span>
          <input className="input" type="email" value={form.email} onChange={set('email')} required autoComplete="email" />
        </label>
        <label className="field">
          <span>Password</span>
          <input className="input" type="password" value={form.password} onChange={set('password')} required minLength={10} autoComplete="new-password" />
          <div className="hint">At least 10 characters.</div>
        </label>
        <button className="btn primary" style={{ width: '100%', justifyContent: 'center' }} disabled={busy}>
          {busy ? 'Setting up…' : 'Create account'}
        </button>
        <ErrorText error={error} />
      </form>
    </div>
  )
}

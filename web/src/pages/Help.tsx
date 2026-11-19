import type { ReactNode } from 'react'
import { NavLink, Navigate, Route, Routes, useLocation } from 'react-router-dom'
import { Printer } from 'lucide-react'
import { canAdmin, canAudit, type Me } from '../api'
import { Access, Audit, DesktopAdmin, Server, SignIn, Sources } from './help/Admin'
import { Models } from './help/Models'
import { serverAddress } from './help/parts'
import { Api, Chatting, DesktopApp } from './help/User'

interface Topic {
  slug: string
  title: string
  /** for administrators and auditors */
  admin: boolean
  body: ReactNode
}

/** Help for everyone, and for administrators how to set Agora up. Prints as a document. */
export function HelpPage({ me }: { me: Me }) {
  const oversight = canAudit(me.user)
  const all: Topic[] = [
    { slug: 'chat', title: 'Chatting', admin: false, body: <Chatting me={me} /> },
    { slug: 'desktop-app', title: 'Agora Desktop', admin: false, body: <DesktopApp me={me} /> },
    { slug: 'api', title: 'Using the API', admin: false, body: <Api /> },
    { slug: 'models', title: 'Ways to run models', admin: true, body: <Models admin={canAdmin(me.user)} /> },
    { slug: 'sign-in', title: 'Accounts and single sign-on', admin: true, body: <SignIn /> },
    { slug: 'access', title: 'Groups, access and budgets', admin: true, body: <Access /> },
    { slug: 'sources', title: 'Web search and knowledge', admin: true, body: <Sources /> },
    { slug: 'audit', title: 'Audit and retention', admin: true, body: <Audit /> },
    { slug: 'desktop', title: 'Agora Desktop: grants and control', admin: true, body: <DesktopAdmin /> },
    { slug: 'server', title: 'Running the server', admin: true, body: <Server /> },
  ]
  const topics = all.filter((t) => oversight || !t.admin)
  const everything = useLocation().pathname.endsWith('/all')
  const link = (t: Topic) => (
    <NavLink key={t.slug} to={`/help/${t.slug}`} className={({ isActive }) => `help-link${isActive ? ' active' : ''}`}>
      {t.title}
    </NavLink>
  )
  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-head">
          <div>
            <h1>Help</h1>
            <div className="muted">{oversight ? 'Using Agora, and setting it up' : 'Using Agora'}</div>
          </div>
          <div className="row no-print" style={{ marginLeft: 'auto' }}>
            {!everything && (
              <NavLink to="/help/all" className="btn" title="Every topic on one page, for printing">
                All on one page
              </NavLink>
            )}
            <button className="btn" onClick={() => window.print()} title="Your browser's print dialog can save this as a PDF">
              <Printer size={15} /> Print or save as PDF
            </button>
          </div>
        </div>
        <div className="help">
          <nav className="help-nav no-print" aria-label="Help topics">
            {oversight && <div className="help-nav-head">For everyone</div>}
            {topics.filter((t) => !t.admin).map(link)}
            {oversight && <div className="help-nav-head">For administrators</div>}
            {topics.filter((t) => t.admin).map(link)}
          </nav>
          <div className="guide">
            <Routes>
              <Route index element={<Navigate to={topics[0].slug} replace />} />
              {topics.map((t) => (
                <Route
                  key={t.slug}
                  path={t.slug}
                  element={
                    <article>
                      <h1 className="help-title">{t.title}</h1>
                      {t.body}
                    </article>
                  }
                />
              ))}
              <Route
                path="all"
                element={
                  <>
                    <h1 className="print-only help-cover">{(serverAddress() === window.location.origin && me.organization) || 'Agora'}: help</h1>
                    {topics.map((t) => (
                      <article key={t.slug} className="help-chapter">
                        <h1 className="help-title">{t.title}</h1>
                        {t.body}
                      </article>
                    ))}
                  </>
                }
              />
              <Route path="*" element={<Navigate to="/help" replace />} />
            </Routes>
          </div>
        </div>
      </div>
    </div>
  )
}

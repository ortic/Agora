import type { ReactNode } from 'react'

export function Code({ children }: { children: string }) {
  return (
    <div className="code-block guide-code">
      <pre>{children}</pre>
    </div>
  )
}

export function Section({ id, title, children }: { id?: string; title: string; children: ReactNode }) {
  return (
    <section id={id} className="card guide-section">
      <h2 style={{ marginTop: 0 }}>{title}</h2>
      {children}
    </section>
  )
}

/** A table of settings: name, meaning. */
export function Options({ head, rows }: { head: [string, string]; rows: [ReactNode, ReactNode][] }) {
  return (
    <div className="table-wrap">
      <table className="data guide-options">
        <thead>
          <tr>
            <th>{head[0]}</th>
            <th>{head[1]}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([a, b], i) => (
            <tr key={i}>
              <td>{a}</td>
              <td>{b}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** This server's address, for the examples (`?address=example` shows a neutral one, for a document that is passed on). */
export function serverAddress(): string {
  return new URLSearchParams(window.location.search).get('address') === 'example' ? 'https://agora.example.com' : window.location.origin
}

import { useEffect, useState, type ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import rehypeHighlight from 'rehype-highlight'
import { Check, Copy, X } from 'lucide-react'

export type Theme = 'system' | 'light' | 'dark'

export function getTheme(): Theme {
  try {
    return (localStorage.getItem('agora-theme') as Theme) || 'system'
  } catch {
    return 'system'
  }
}

export function applyTheme(t: Theme = getTheme()) {
  const root = document.documentElement
  if (t === 'system') root.removeAttribute('data-theme')
  else root.setAttribute('data-theme', t)
  try {
    localStorage.setItem('agora-theme', t)
  } catch {
    /* private mode */
  }
}

export function CopyButton({ text, label }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className="icon-btn"
      title={label ?? 'Copy'}
      aria-label={label ?? 'Copy'}
      onClick={() => {
        navigator.clipboard.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1500)
        })
      }}
    >
      {done ? <Check size={15} /> : <Copy size={15} />}
    </button>
  )
}

function CodeBlock({ className, children }: { className?: string; children?: ReactNode }) {
  const lang = /language-([\w-]+)/.exec(className ?? '')?.[1]
  const text = textOf(children)
  return (
    <div className="code-block">
      <div className="code-head">
        <span>{lang ?? 'code'}</span>
        <CopyButton text={text.replace(/\n$/, '')} label="Copy code" />
      </div>
      <pre>
        <code className={className}>{children}</code>
      </pre>
    </div>
  )
}

function textOf(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return ''
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (typeof node === 'object' && 'props' in node) return textOf((node as { props: { children?: ReactNode } }).props.children)
  return ''
}

/** Markdown as the models write it: GitHub flavour, highlighted code, no raw HTML. */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: false, ignoreMissing: true }]]}
        components={{
          pre: ({ children }) => {
            const child = Array.isArray(children) ? children[0] : children
            const props = (child as { props?: { className?: string; children?: ReactNode } })?.props ?? {}
            return <CodeBlock className={props.className}>{props.children}</CodeBlock>
          },
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}

export function Dialog({ title, onClose, children, wide }: { title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`dialog${wide ? ' wide' : ''}`} role="dialog" aria-modal="true" aria-label={title}>
        <div className="row" style={{ marginBottom: 12 }}>
          <h2 className="grow" style={{ margin: 0 }}>
            {title}
          </h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        {children}
      </div>
    </div>
  )
}

export function Avatar({ name }: { name: string }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]!.toUpperCase())
    .join('')
  return <span className="avatar" aria-hidden="true">{initials || '?'}</span>
}

export function ErrorText({ error }: { error: unknown }) {
  if (!error) return null
  return <div className="banner error" style={{ marginTop: 10 }}>{error instanceof Error ? error.message : String(error)}</div>
}

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

/** Bar chart of a series (days). */
export function Bars({ values, labels, format }: { values: number[]; labels: string[]; format: (n: number) => string }) {
  const max = Math.max(...values, 0) || 1
  return (
    <div>
      <div className="bars" role="img" aria-label="Chart by day">
        {values.map((v, i) => (
          <div key={i} className="bar" style={{ height: `${(v / max) * 100}%` }} title={`${labels[i]}: ${format(v)}`} />
        ))}
      </div>
      <div className="bars-axis">
        <span>{labels[0]}</span>
        <span>{labels[labels.length - 1]}</span>
      </div>
    </div>
  )
}

export function Meter({ value, max }: { value: number; max: number | null }) {
  if (!max) return null
  const pct = Math.min(100, (value / max) * 100)
  return (
    <div className={`meter${pct >= 100 ? ' over' : pct >= 80 ? ' warn' : ''}`} role="meter" aria-valuenow={value} aria-valuemax={max}>
      <div style={{ width: `${pct}%` }} />
    </div>
  )
}

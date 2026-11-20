import { useRef, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { FileText, FolderKanban, Image, MessageSquarePlus, Plus, Trash2, Upload, Users } from 'lucide-react'
import { ago, api, plural, type ChatSummary, type Me, type Project, type ProjectSummary } from '../api'
import { Dialog, ErrorText } from '../ui'

const size = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.ceil(n / 1000))} kB`)

const sharedWith = (p: { access: string; mine: boolean; owner: string }) =>
  !p.mine ? `shared by ${p.owner}` : p.access === 'everyone' ? 'shared with everyone' : p.access === 'groups' ? 'shared with groups' : 'only you'

/** `/projects`: the person's projects and those shared with them. */
export function ProjectsPage() {
  const list = useQuery({ queryKey: ['projects'], queryFn: () => api.get<ProjectSummary[]>('/projects') })
  const [creating, setCreating] = useState(false)
  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 900 }}>
        <div className="page-head">
          <div className="grow">
            <h1>Projects</h1>
            <div className="muted">Chats that belong together, with instructions and files the model gets with every message.</div>
          </div>
          <button className="btn primary" onClick={() => setCreating(true)}>
            <Plus size={16} /> New project
          </button>
        </div>
        <ErrorText error={list.error} />
        {list.data?.length === 0 && (
          <div className="card muted">
            No projects yet. A project is worth making when you keep explaining the same background: put it in the project's instructions and files once, and every chat in the project starts with it.
          </div>
        )}
        <div className="project-grid">
          {list.data?.map((p) => (
            <Link key={p.id} to={`/projects/${p.id}`} className="project-card">
              <div className="name">
                <FolderKanban size={16} /> <span>{p.name}</span>
              </div>
              {p.description && <div className="muted desc">{p.description}</div>}
              <div className="faint meta">
                {plural(p.chats, 'chat')} · {plural(p.files, 'file')} · {sharedWith(p)}
              </div>
            </Link>
          ))}
        </div>
      </div>
      {creating && <CreateDialog onClose={() => setCreating(false)} />}
    </div>
  )
}

function CreateDialog({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient()
  const navigate = useNavigate()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const create = useMutation({
    mutationFn: () => api.post<Project>('/projects', { name, description }),
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: ['projects'] })
      navigate(`/projects/${p.id}`)
    },
  })
  return (
    <Dialog title="New project" onClose={onClose}>
      <label className="field">
        <span>Name</span>
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} autoFocus maxLength={120} onKeyDown={(e) => e.key === 'Enter' && name.trim() && create.mutate()} />
      </label>
      <label className="field">
        <span>What it is for (optional)</span>
        <input className="input" value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} />
      </label>
      <ErrorText error={create.error} />
      <div className="actions">
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={!name.trim() || create.isPending} onClick={() => create.mutate()}>Create</button>
      </div>
    </Dialog>
  )
}

function readBase64(f: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onerror = () => reject(new Error(`${f.name} could not be read`))
    r.onload = () => resolve(String(r.result).split(',')[1] ?? '')
    r.readAsDataURL(f)
  })
}

/** `/projects/{id}`: instructions, files, sharing and the person's chats in the project. */
export function ProjectPage({ me }: { me: Me }) {
  const { id } = useParams()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const project = useQuery({ queryKey: ['project', id], queryFn: () => api.get<Project>(`/projects/${id}`) })
  const chats = useQuery({ queryKey: ['chats', 'project', id], queryFn: () => api.get<ChatSummary[]>(`/chats?project=${id}`) })
  const done = (p: Project) => {
    qc.setQueryData(['project', id], p)
    qc.invalidateQueries({ queryKey: ['projects'] })
    qc.invalidateQueries({ queryKey: ['chats'] })
  }
  const update = useMutation({ mutationFn: (b: Record<string, unknown>) => api.patch<Project>(`/projects/${id}`, b), onSuccess: done })
  const remove = useMutation({
    mutationFn: () => api.del(`/projects/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['projects'] })
      qc.invalidateQueries({ queryKey: ['chats'] })
      navigate('/projects')
    },
  })
  const [uploadError, setUploadError] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  const removeFile = useMutation({ mutationFn: (file: string) => api.del<Project>(`/projects/${id}/files/${file}`), onSuccess: done })
  const picker = useRef<HTMLInputElement>(null)

  async function upload(files: FileList | File[]) {
    setUploadError(null)
    setUploading(true)
    try {
      for (const f of Array.from(files)) {
        if (f.size > 21 * 1024 * 1024) throw new Error(`${f.name}: too large`)
        done(await api.post<Project>(`/projects/${id}/files`, { name: f.name, mediaType: f.type, data: await readBase64(f) }))
      }
    } catch (e) {
      setUploadError(e instanceof Error ? e.message : String(e))
    } finally {
      setUploading(false)
    }
  }

  if (project.isError) {
    return <div className="empty-state"><div className="inner"><h1>Project not found</h1><p className="muted">It may have been deleted, or is no longer shared with you.</p></div></div>
  }
  const p = project.data
  if (!p) return null
  const textChars = p.files.reduce((a, f) => a + (f.chars ?? 0), 0)
  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 900 }}>
        <div className="faint" style={{ fontSize: 13, marginBottom: 6 }}>
          <Link to="/projects">Projects</Link> /
        </div>
        <div className="page-head">
          <div className="grow" style={{ minWidth: 0 }}>
            {p.mine ? <Editable key={p.name} value={p.name} className="project-name" label="Project name" onSave={(name) => update.mutate({ name })} /> : <h1>{p.name}</h1>}
            {p.mine ? (
              <Editable key={p.description} value={p.description} className="muted" label="What it is for" placeholder="What it is for (optional)" onSave={(description) => update.mutate({ description })} />
            ) : (
              <div className="muted">{p.description}</div>
            )}
            <div className="faint" style={{ fontSize: 13, marginTop: 4 }}>{p.mine ? `Yours · ${sharedWith(p)}` : `Shared by ${p.owner}: you can use it, they change it`}</div>
          </div>
          <Link to={`/?project=${p.id}`} className="btn primary">
            <MessageSquarePlus size={16} /> New chat in this project
          </Link>
        </div>
        <ErrorText error={update.error} />

        <h2>Your chats in this project</h2>
        {chats.data?.length === 0 && <div className="faint">None yet.</div>}
        {!!chats.data?.length && (
          <div className="table-wrap">
            <table className="data">
              <tbody>
                {chats.data.map((c) => (
                  <tr key={c.id}>
                    <td><Link to={`/c/${c.id}`}>{c.title}</Link></td>
                    <td className="faint" style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>{ago(c.updatedAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <h2>Instructions</h2>
        {!me.allowUserSystemPrompt ? (
          <div className="banner">Your organization doesn't allow instructions of your own, so a project's instructions aren't given to the model. Its files are.</div>
        ) : p.mine ? (
          <Instructions key={p.instructions} initial={p.instructions} saving={update.isPending} onSave={(instructions) => update.mutate({ instructions })} />
        ) : p.instructions ? (
          <pre className="project-instructions">{p.instructions}</pre>
        ) : (
          <div className="faint">None.</div>
        )}

        <h2>Files</h2>
        <p className="muted" style={{ marginTop: 0 }}>
          The model gets all of them with every message in the project, so they count toward every answer's cost. Text files of any kind (Markdown, CSV, code), images and PDFs.
          {p.files.length > 0 && ` Text used: ${textChars.toLocaleString('en-US')} of ${p.limits.textChars.toLocaleString('en-US')} characters.`}
        </p>
        {p.files.length > 0 && (
          <div className="table-wrap" style={{ marginBottom: 10 }}>
            <table className="data">
              <tbody>
                {p.files.map((f) => (
                  <tr key={f.id}>
                    <td>
                      {f.chars == null && f.mediaType.startsWith('image/') ? <Image size={14} style={{ verticalAlign: -2, marginRight: 6 }} /> : <FileText size={14} style={{ verticalAlign: -2, marginRight: 6 }} />}
                      <a href={`/api/projects/${p.id}/files/${f.id}`} target="_blank" rel="noopener noreferrer">{f.name}</a>
                    </td>
                    <td className="faint" style={{ whiteSpace: 'nowrap' }}>{f.chars != null ? `${f.chars.toLocaleString('en-US')} characters` : f.mediaType === 'application/pdf' ? 'PDF' : 'image'}</td>
                    <td className="faint" style={{ whiteSpace: 'nowrap', textAlign: 'right' }}>{size(f.size)}</td>
                    {p.mine && (
                      <td style={{ textAlign: 'right', width: 40 }}>
                        <button className="icon-btn" title={`Remove ${f.name}`} aria-label={`Remove ${f.name}`} onClick={() => removeFile.mutate(f.id)}>
                          <Trash2 size={14} />
                        </button>
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {p.mine && (
          <div
            className="dropzone"
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => {
              e.preventDefault()
              if (e.dataTransfer.files.length) upload(e.dataTransfer.files)
            }}
          >
            <input ref={picker} type="file" multiple hidden onChange={(e) => {
              if (e.target.files?.length) upload(e.target.files)
              e.target.value = ''
            }} />
            <button className="btn" disabled={uploading} onClick={() => picker.current?.click()}>
              <Upload size={15} /> {uploading ? 'Adding…' : 'Add files'}
            </button>
            <span className="faint">or drop them here</span>
          </div>
        )}
        {!p.mine && p.files.length === 0 && <div className="faint">None.</div>}
        {uploadError && <div className="banner error" style={{ marginTop: 10 }}>{uploadError}</div>}
        <ErrorText error={removeFile.error} />

        {p.mine && (
          <>
            <h2><Users size={16} style={{ verticalAlign: -2 }} /> Sharing</h2>
            <p className="muted" style={{ marginTop: 0 }}>
              People you share with can read the instructions and files and start chats of their own in the project. Nobody sees another person's chats, and only you can change the project.
            </p>
            {(['private', 'everyone', 'groups'] as const).map((a) => (
              <label key={a} className="check" style={{ marginBottom: 6 }}>
                <input type="radio" name="access" checked={p.access === a} onChange={() => update.mutate({ access: a })} disabled={a === 'groups' && !p.groups?.length} />
                <span>
                  {a === 'private' ? 'Only me' : a === 'everyone' ? 'Everyone in the organization' : 'Members of these groups'}
                  {a === 'groups' && !p.groups?.length && <span className="faint"> (you are in no group)</span>}
                </span>
              </label>
            ))}
            {p.access === 'groups' && (
              <div className="row" style={{ margin: '4px 0 0 24px' }}>
                {p.groups?.map((g) => {
                  const on = !!p.groupIds?.includes(g.id)
                  return (
                    <label key={g.id} className="check" style={{ marginBottom: 0 }}>
                      <input
                        type="checkbox"
                        checked={on}
                        onChange={() => update.mutate({ groupIds: on ? p.groupIds!.filter((x) => x !== g.id) : [...(p.groupIds ?? []), g.id] })}
                      />
                      {g.name}
                    </label>
                  )
                })}
              </div>
            )}

            <h2>Delete</h2>
            <button
              className="btn danger"
              onClick={() => {
                if (confirm(`Delete the project “${p.name}” with its instructions and files? Its chats stay, without the project.`)) remove.mutate()
              }}
            >
              <Trash2 size={15} /> Delete this project
            </button>
            <ErrorText error={remove.error} />
          </>
        )}
      </div>
    </div>
  )
}

function Instructions({ initial, saving, onSave }: { initial: string; saving: boolean; onSave: (s: string) => void }) {
  const [s, setS] = useState(initial)
  return (
    <>
      <textarea
        className="input"
        rows={7}
        value={s}
        aria-label="Project instructions"
        onChange={(e) => setS(e.target.value)}
        placeholder="What the model should know and do in every chat of this project: the goal, who it is for, how to answer."
      />
      <div className="row" style={{ marginTop: 8 }}>
        <button className="btn primary" disabled={saving || s === initial} onClick={() => onSave(s)}>Save instructions</button>
        {s !== initial && <span className="faint">not saved yet</span>}
      </div>
    </>
  )
}

/** Text that turns into an input when clicked. */
function Editable({ value, onSave, className, label, placeholder }: { value: string; onSave: (s: string) => void; className?: string; label: string; placeholder?: string }) {
  const [editing, setEditing] = useState(false)
  const [s, setS] = useState(value)
  if (!editing) {
    return (
      <button className={`editable ${className ?? ''}`} title={`Change: ${label.toLowerCase()}`} onClick={() => setEditing(true)}>
        {value || <span className="faint">{placeholder}</span>}
      </button>
    )
  }
  return (
    <input
      className="input"
      autoFocus
      value={s}
      aria-label={label}
      onChange={(e) => setS(e.target.value)}
      onBlur={() => {
        setEditing(false)
        if (s.trim() !== value && (s.trim() || placeholder)) onSave(s)
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
        if (e.key === 'Escape') {
          setS(value)
          setEditing(false)
        }
      }}
    />
  )
}

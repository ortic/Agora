import type { ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { api } from '../../api'
import { Code } from './parts'

interface Provider {
  id: string
  name: string
  kind: 'anthropic' | 'openai' | 'claude_code'
  baseUrl: string
  enabled: boolean
  models: number
}

const local = (url: string) => /\/\/(127\.|localhost|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|host\.docker\.internal|\[::1\])/.test(url)

function Way({ id, title, state, children }: { id: string; title: string; state: ReactNode; children: ReactNode }) {
  return (
    <section id={id} className="card guide-section">
      <div className="row">
        <h2 className="grow" style={{ margin: 0 }}>{title}</h2>
        {state}
      </div>
      {children}
    </section>
  )
}

/** The ways to give Agora its models. `admin`: may read the providers (for the "in use" marks). */
export function Models({ admin }: { admin: boolean }) {
  const providers = useQuery({ queryKey: ['admin-providers'], queryFn: () => api.get<Provider[]>('/admin/providers'), enabled: admin })
  const list = providers.data ?? []
  const state = (match: (p: Provider) => boolean) => {
    if (!admin) return null
    const mine = list.filter((p) => match(p) && p.enabled)
    const models = mine.reduce((n, p) => n + p.models, 0)
    if (mine.length === 0) return <span className="badge no-print">not set up</span>
    return <span className="badge accent no-print">{models === 0 ? 'connected, no models yet' : `in use: ${models} model${models === 1 ? '' : 's'}`}</span>
  }
  return (
    <>
      <p className="muted">Agora doesn't run models itself. It sends each conversation to a model somewhere else, and you choose where.</p>
      <section className="card guide-section">
        <h2 style={{ marginTop: 0 }}>The three ways at a glance</h2>
        <div className="table-wrap">
          <table className="data guide-table">
            <thead>
              <tr>
                <th></th>
                <th><a href="#claude-api">Claude API</a></th>
                <th><a href="#claude-code">Claude Code (claude -p)</a></th>
                <th><a href="#ollama">Ollama / your own hardware</a></th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <th>Where the model runs</th>
                <td>At Anthropic</td>
                <td>At Anthropic; the Claude Code program runs on the Agora server</td>
                <td>On a computer of yours with a GPU</td>
              </tr>
              <tr>
                <th>What you need</th>
                <td>An Anthropic API key</td>
                <td>Claude Code installed on the server, signed in or with an API key</td>
                <td>A machine with enough GPU memory, reachable from the Agora server</td>
              </tr>
              <tr>
                <th>Who pays, and how</th>
                <td>Your Anthropic account, per token</td>
                <td>The account Claude Code is signed in with (a subscription), or an API key per token</td>
                <td>Nobody per message; you pay for the hardware and power</td>
              </tr>
              <tr>
                <th>Does text leave your network?</th>
                <td>Yes, to Anthropic</td>
                <td>Yes, to Anthropic</td>
                <td>No</td>
              </tr>
              <tr>
                <th>Who runs the tool loop</th>
                <td>Agora</td>
                <td>Claude Code (its own planning, sub-agents and context handling)</td>
                <td>Agora</td>
              </tr>
              <tr>
                <th>Work on code with Agora Desktop</th>
                <td>Very good</td>
                <td>Very good; closest to using Claude Code itself</td>
                <td>Depends on the model; small models use tools poorly</td>
              </tr>
              <tr>
                <th>Best for</th>
                <td>An organization: clear billing, several people at once</td>
                <td>One person or a small team that already uses Claude Code</td>
                <td>Confidential data, and work that must stay offline</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p className="muted" style={{ marginBottom: 0 }}>
          You can use all of them side by side: each becomes a provider under <Link to="/admin/models">Models</Link>, each model can be limited to groups, and people pick a model per chat. A fourth way, other hosted services with an OpenAI-compatible API, is <a href="#other">described below</a>.
        </p>
      </section>

      <Way id="claude-api" title="1. Claude through the Anthropic API" state={state((p) => p.kind === 'anthropic')}>
        <p>Agora talks to Anthropic directly. This is the simplest setup and the one with the clearest costs.</p>
        <ol>
          <li>Create an API key in the Anthropic Console (console.anthropic.com → API keys).</li>
          <li>In Agora: <Link to="/admin/models">Models</Link> → <b>Connect a provider</b> → <b>Anthropic (Claude)</b>, and paste the key. It is stored encrypted with the server's secret key.</li>
          <li><b>Add models</b>: Agora lists the models your key can use. Add the ones you want and check their prices.</li>
          <li>Set a default model under <Link to="/admin/settings">Settings</Link>, and a monthly budget per person if several people use it.</li>
        </ol>
        <h3>What you get</h3>
        <ul>
          <li>Claude's thinking shown as a collapsible thought process, effort levels, and prompt caching (long chats cost less).</li>
          <li>Claude's own web search, or Agora's search engine: your choice under Settings.</li>
          <li>Images and PDFs in messages, and the full set of Agora Desktop tools.</li>
          <li>Token counts and cost under every answer, per person in <Link to="/admin/usage">Usage</Link>.</li>
        </ul>
      </Way>

      <Way id="claude-code" title="2. Claude Code on the server (claude -p)" state={state((p) => p.kind === 'claude_code')}>
        <p>
          The server runs the Claude Code program for each answer and relays what it writes. You get Claude Code's own way of working (its planning, sub-agents and handling of long sessions) inside Agora's chats.
        </p>
        <ol>
          <li>
            Install Claude Code on the machine Agora runs on, for the account the Agora server runs as:
            <Code>{'curl -fsSL https://claude.ai/install.sh | bash\nclaude --version'}</Code>
          </li>
          <li>
            Give it an account. Either sign in once as that same account (run <code>claude</code>, then <code>/login</code>), or skip this and put an Anthropic API key on the provider in the next step.
          </li>
          <li>In Agora: <Link to="/admin/models">Models</Link> → <b>Connect a provider</b> → <b>Claude Code on this server</b>. Leave the URL empty; the API key is optional.</li>
          <li><b>Add models</b> by the names Claude Code takes: <code>sonnet</code>, <code>opus</code>, <code>haiku</code>, or a full model name.</li>
        </ol>
        <h3>How it behaves</h3>
        <ul>
          <li>Claude Code gets none of its own tools for files, commands or the web, so nothing runs on the server. It gets Agora's tools instead: web search, your knowledge sources, and the tools on a person's computer through Agora Desktop. Grants, confirmations and the record of every call are the same as with other models.</li>
          <li>The server account's own Claude Code settings, hooks and commands are not used.</li>
          <li>A chat continues the Claude Code session of its previous answer. Sessions are kept by Claude Code in the home folder of the server's account.</li>
          <li>Without prices on the model, the cost Claude Code reports is shown and counts toward budgets. On a subscription that figure is computed, not billed.</li>
        </ul>
        <h3>Things to know</h3>
        <ul>
          <li>Without an API key, everyone's answers run on the one account Claude Code is signed in with. That suits one person; check Anthropic's terms before serving several people from a personal subscription.</li>
          <li>The Agora server must be able to reach itself on its listen address: Claude Code connects back to it for its tools.</li>
          <li>The Docker image doesn't contain Claude Code. Run the server directly on a machine that has it, or build an image that adds it.</li>
          <li>Server settings: <code>AGORA_CLAUDE_BIN</code> (the program, default <code>claude</code>) and <code>AGORA_CLAUDE_DIR</code> (where it runs, default <code>./data/claude-code</code>).</li>
        </ul>
      </Way>

      <Way id="ollama" title="3. Ollama, or another model server on your own hardware" state={state((p) => p.kind === 'openai' && local(p.baseUrl))}>
        <p>
          The model runs on a computer of yours. Nothing leaves your network, and there is no cost per message. Any server with an OpenAI-compatible API works; Ollama is the easiest to start with.
        </p>
        <h3>On the same machine as Agora</h3>
        <Code>{'ollama serve                 # listens on 127.0.0.1:11434\nollama pull qwen3            # or llama3.3, mistral-small, gpt-oss:20b, …'}</Code>
        <p>
          In Agora: <Link to="/admin/models">Models</Link> → <b>Connect a provider</b> → <b>Ollama on this machine</b> (<code>http://127.0.0.1:11434/v1</code>), then <b>Add models</b>.
        </p>
        <h3>On another computer (the Agora server stays small)</h3>
        <p>Run Ollama on the machine with the GPU and let it listen on the network:</p>
        <Code>{'OLLAMA_HOST=0.0.0.0 OLLAMA_CONTEXT_LENGTH=32768 ollama serve'}</Code>
        <p>
          In Agora, connect a provider of the kind <b>Other OpenAI-compatible</b> with the address <code>http://&lt;that-machine&gt;:11434/v1</code>. You can connect several such machines.
        </p>
        <ul>
          <li>Ollama has no sign-in of its own. Let only the Agora server reach that port (a firewall rule or a private network), or put a proxy with an API key in front and enter the key on the provider.</li>
          <li>If Agora runs in Docker and Ollama on the same host, <code>127.0.0.1</code> inside the container is the container itself: use the host's address instead.</li>
        </ul>
        <h3>Settings that matter</h3>
        <ul>
          <li><b>Context length.</b> Set <code>OLLAMA_CONTEXT_LENGTH</code>: otherwise Ollama sizes it from the GPU memory, which can exhaust it. Enter the same number as the model's context window in Agora, because Agora summarizes long conversations when they approach it.</li>
          <li><b>Tools.</b> Turn the "tools" capability off for models that can't call tools reliably. Small models work poorly with Agora Desktop's coding tools.</li>
          <li><b>Many people at once.</b> vLLM (<code>vllm serve &lt;model&gt;</code>, address <code>http://host:8000/v1</code>) handles concurrent requests better than Ollama.</li>
          <li>Say in the model's description that it is the one for confidential data: people see that in the model picker.</li>
        </ul>
      </Way>

      <Way id="other" title="Other hosted services" state={state((p) => p.kind === 'openai' && !local(p.baseUrl))}>
        <p>
          OpenAI, OpenRouter, an Azure gateway, LiteLLM and similar services speak the same OpenAI-compatible API as Ollama. Connect them under <Link to="/admin/models">Models</Link> with their address (up to and including <code>/v1</code>) and key. Agora's tool loop, budgets and records work the same; Claude-only features such as the shown thought process and prompt caching don't apply.
        </p>
      </Way>

      <section className="card guide-section">
        <h2 style={{ marginTop: 0 }}>Using several at once</h2>
        <ul>
          <li><b>Who sees which model:</b> each model is open to everyone or to chosen groups (<Link to="/admin/groups">Groups</Link>).</li>
          <li><b>Spending:</b> a monthly budget in USD per person, per group or as a default for everyone; a request limit per minute. Both under <Link to="/admin/settings">Settings</Link> and Groups.</li>
          <li><b>Chat titles:</b> pick a small, cheap model for writing titles under Settings, so the large one isn't used for that.</li>
          <li><b>A typical mix:</b> a Claude model for demanding work, a local model for confidential data, and budgets so that the expensive one is used with care.</li>
        </ul>
        <h3>Checking that a model works</h3>
        <p style={{ marginBottom: 0 }}>
          Start a new chat, pick the model and send a short message. If something is wrong, the reason is shown under the answer (a wrong key, an unreachable address, a model name the provider doesn't know). Every call, with its tokens, cost and errors, is listed under <Link to="/admin/usage">Usage</Link>.
        </p>
      </section>
    </>
  )
}

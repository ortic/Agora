import { Link } from 'react-router-dom'
import { Section } from './parts'

type Tone = 'plain' | 'inside' | 'outside' | 'core'

function Box({ x, y, w, h, title, sub, tone = 'plain' }: { x: number; y: number; w: number; h: number; title: string; sub?: string; tone?: Tone }) {
  return (
    <g>
      <rect x={x} y={y} width={w} height={h} rx={8} className={`chart-box ${tone}`} />
      <text x={x + 12} y={y + (sub ? 22 : h / 2 + 5)} className="chart-title">{title}</text>
      {sub && <text x={x + 12} y={y + 40} className="chart-sub">{sub}</text>}
    </g>
  )
}

function Group({ x, y, w, h, label }: { x: number; y: number; w: number; h: number; label: string }) {
  return (
    <g>
      <rect x={x} y={y} width={w} height={h} rx={12} className="chart-group" />
      <text x={x + 14} y={y + 20} className="chart-group-label">{label}</text>
    </g>
  )
}

/** A line with an arrowhead at its end. `points`: "x,y x,y …". */
function Arrow({ points, dashed, label, lx, ly }: { points: string; dashed?: boolean; label?: string; lx?: number; ly?: number }) {
  return (
    <g>
      <polyline points={points} className={`chart-line${dashed ? ' dashed' : ''}`} markerEnd="url(#chart-arrow)" />
      {label && <text x={lx} y={ly} textAnchor="middle" className="chart-label">{label}</text>}
    </g>
  )
}

/** How the parts fit together: people and their computers, the server, where the models run. */
export function Chart() {
  return (
    <div className="chart-wrap">
      <svg viewBox="0 0 1000 640" className="chart" role="img" aria-labelledby="chart-title chart-desc">
        <title id="chart-title">How Agora fits together</title>
        <desc id="chart-desc">
          People use Agora in a web browser, through the Agora Desktop app or from other programs. All of them talk to the Agora server, which stores everything in PostgreSQL and sends conversations to models: Claude at Anthropic through the API or through Claude Code, Ollama or vLLM on the same server or on another machine, or other hosted services. The server can also use an identity provider, a web search engine and a wiki.
        </desc>
        <defs>
          <marker id="chart-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" className="chart-head" />
          </marker>
        </defs>

        <Group x={10} y={30} w={300} h={450} label="People and their computers" />
        <Group x={360} y={30} w={280} h={450} label="Your server" />
        <Group x={690} y={30} w={298} h={450} label="Where the models run" />

        {/* people */}
        <Box x={26} y={62} w={268} h={52} title="Web browser" sub="chats, administration, help" />
        <Box x={26} y={130} w={268} h={226} title="Agora Desktop" sub="the app on a person's computer" />
        <Box x={40} y={184} w={240} h={46} title="Dev container" sub="the project's code and commands" tone="inside" />
        <Box x={40} y={236} w={240} h={46} title="Browser window" sub="with a profile of its own" tone="inside" />
        <Box x={40} y={288} w={240} h={46} title="Local connectors" sub="MCP servers for local apps" tone="inside" />
        <text x={40} y={349} className="chart-sub">also without a window: agora-agent</text>
        <Box x={26} y={372} w={268} h={52} title="Other programs" sub="editors and scripts, OpenAI-style API" />

        {/* the server */}
        <rect x={376} y={62} width={248} height={294} rx={8} className="chart-box core" />
        <text x={388} y={84} className="chart-title">Agora</text>
        {['chats and the tool loop', 'who may use which model', 'budgets and usage', 'audit log', 'sign-in'].map((line, i) => (
          <text key={line} x={388} y={110 + i * 20} className="chart-sub">· {line}</text>
        ))}
        <Box x={390} y={296} w={220} h={48} title="Claude Code (claude -p)" sub="optional: its own agent loop" />
        <Box x={376} y={372} w={248} h={52} title="PostgreSQL" sub="everything is stored here" />
        <polyline points="500,356 500,372" className="chart-line" />

        {/* the models */}
        <Box x={706} y={62} w={268} h={58} title="Claude at Anthropic" sub="the Anthropic API, paid per token" tone="outside" />
        <Box x={706} y={136} w={268} h={58} title="Ollama or vLLM on this server" sub="needs a GPU here" tone="inside" />
        <Box x={706} y={210} w={268} h={58} title="Ollama or vLLM on another machine" sub="a GPU server of yours" tone="inside" />
        <Box x={706} y={284} w={268} h={58} title="Other hosted services" sub="OpenAI, OpenRouter, Azure, LiteLLM" tone="outside" />
        <Box x={706} y={358} w={268} h={58} title="Claude, through Claude Code" sub="a subscription, or an API key" tone="outside" />

        {/* people to the server */}
        <Arrow points="294,88 376,88" label="HTTPS" lx={335} ly={82} />
        <Arrow points="294,200 376,200" label="messages" lx={335} ly={194} />
        <Arrow points="376,252 294,252" dashed label="tool calls" lx={335} ly={246} />
        <Arrow points="294,398 335,398 335,278 376,278" label="/api/v1" lx={335} ly={414} />

        {/* the server to the models */}
        <Arrow points="624,91 706,91" />
        <Arrow points="624,165 706,165" />
        <Arrow points="624,239 706,239" />
        <Arrow points="624,278 652,278 652,313 706,313" />
        <Arrow points="610,326 680,326 680,387 706,387" />

        {/* what else the server can use */}
        <text x={10} y={508} className="chart-group-label">Optional, when you set them up</text>
        <polyline points="500,480 500,500" className="chart-line" />
        <polyline points="235,500 765,500" className="chart-line" />
        <Arrow points="235,500 235,520" />
        <Arrow points="500,500 500,520" />
        <Arrow points="765,500 765,520" />
        <Box x={120} y={520} w={230} h={58} title="Identity provider" sub="OIDC, SAML, LDAP, GitHub" />
        <Box x={385} y={520} w={230} h={58} title="Web search" sub="SearXNG, Brave, Serper, Tavily" />
        <Box x={650} y={520} w={230} h={58} title="Wiki" sub="WikiCraft, as a knowledge source" />

        {/* legend */}
        <rect x={10} y={604} width={16} height={16} rx={4} className="chart-box inside" />
        <text x={32} y={617} className="chart-sub">stays on your hardware</text>
        <rect x={190} y={604} width={16} height={16} rx={4} className="chart-box outside" />
        <text x={212} y={617} className="chart-sub">conversations leave your network</text>
        <polyline points="440,612 480,612" className="chart-line" markerEnd="url(#chart-arrow)" />
        <text x={488} y={617} className="chart-sub">requests</text>
        <polyline points="570,612 610,612" className="chart-line dashed" markerEnd="url(#chart-arrow)" />
        <text x={618} y={617} className="chart-sub">tool calls, run on the person's computer</text>
      </svg>
    </div>
  )
}

/** The overview: the chart and how to read it. */
export function Overview({ admin }: { admin: boolean }) {
  return (
    <>
      <Section title="How it fits together">
        <Chart />
        <ul>
          <li><b>Everything goes through the Agora server.</b> The web app, the desktop app and other programs talk only to it; none of them talks to a model directly. That is why access rules, budgets and the audit log apply to all of them alike.</li>
          <li><b>Agora runs no model itself.</b> It sends each conversation to one of the places on the right. Several can be set up at once, and people pick a model per chat.</li>
          <li><b>The model's work is steered on the server.</b> When a model wants to read a file, run a command or click in a browser, the server sends that one step to the person's Agora Desktop app, which carries it out and sends back the result.</li>
          <li><b>What leaves your network</b> is the conversation, and only to the model you chose for it: to Anthropic or another hosted service, or nowhere when the model runs on hardware of your own.</li>
        </ul>
      </Section>
      <Section title="The options on the right">
        <ul>
          <li><b>Claude at Anthropic:</b> the Anthropic API with a key; paid per token.</li>
          <li><b>Ollama or vLLM on this server:</b> the model runs next to Agora; the server needs a GPU.</li>
          <li><b>Ollama or vLLM on another machine:</b> the Agora server stays small, and a GPU machine of yours does the work. It only has to be reachable from the server.</li>
          <li><b>Other hosted services:</b> anything with an OpenAI-compatible API.</li>
          <li><b>Claude, through Claude Code:</b> the server runs the Claude Code program, which brings its own way of working and uses a subscription or an API key.</li>
        </ul>
        <p style={{ marginBottom: 0 }}>
          {admin ? (
            <>
              Step by step for each: <Link to="/help/models">Ways to run models</Link>.
            </>
          ) : (
            'Your administrators decide which of these your organization uses.'
          )}
        </p>
      </Section>
    </>
  )
}

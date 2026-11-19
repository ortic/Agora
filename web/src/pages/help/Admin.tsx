import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { api } from '../../api'
import { Code, Options, Section, serverAddress } from './parts'

interface AuthProvider {
  id: string
  name: string
  type: string
}

/** Accounts, roles and single sign-on. */
export function SignIn() {
  const base = serverAddress()
  const configured = useQuery({
    queryKey: ['auth-providers'],
    queryFn: async () => {
      const v = await api.get<AuthProvider[] | { providers: AuthProvider[] }>('/auth/providers')
      return Array.isArray(v) ? v : (v.providers ?? [])
    },
  })
  const list = configured.data ?? []
  return (
    <>
      <p className="muted">
        People sign in with a local password, through your identity provider, or both. Sign-in providers are part of the server's configuration, not of the admin pages: the server reads them when it starts.
      </p>
      <Section title="Accounts and roles">
        <p>
          <Link to="/admin/users">Users</Link> lists everyone; you can create accounts by hand, change roles, set a budget for one person, reset a password and deactivate an account.
        </p>
        <Options
          head={['Role', 'May']}
          rows={[
            ['owner', 'Everything an admin may. The account created on the first visit.'],
            ['admin', 'Manage models, people, groups, settings and Agora Desktop.'],
            ['auditor', 'Read the audit log, usage and conversations. Changes nothing. Opening a conversation is recorded.'],
            ['member', 'Chat, with the models and features their groups allow.'],
          ]}
        />
      </Section>

      <Section title="Single sign-on: how to set it up">
        <p className="no-print">
          On this server now:{' '}
          {list.length === 0 ? <span className="badge">no sign-in provider configured</span> : list.map((p) => <span key={p.id} className="badge accent" style={{ marginRight: 4 }}>{p.name} ({p.type})</span>)}
        </p>
        <ol>
          <li>Register Agora as an application with your identity provider (the sections below say how, per kind).</li>
          <li>
            Describe the providers as a JSON list and give it to the server in <code>AGORA_AUTH_PROVIDERS</code>, or in a file named by <code>AGORA_AUTH_PROVIDERS_FILE</code> (for example a mounted secret). Each entry has a <code>type</code> and an <code>id</code> of your choice; the id appears in the addresses below.
          </li>
          <li>Set <code>AGORA_BASE_URL</code> to the address people use (here: <code>{base}</code>). Redirect addresses are built from it. Behind HTTPS also set <code>AGORA_SECURE_COOKIES=true</code>.</li>
          <li>Restart the server. A mistake in the list stops it from starting, with a message that names the entry.</li>
          <li>The sign-in page then shows a button per provider. To allow only single sign-on, set <code>AGORA_DISABLE_PASSWORD_LOGIN=true</code> (keep one working provider first).</li>
        </ol>
        <p style={{ marginBottom: 0 }}>Several providers can be configured at once.</p>
      </Section>

      <Section id="oidc" title="OpenID Connect (OAuth 2.0): Microsoft Entra ID, Google, GitLab, Okta, Keycloak, Authentik …">
        <p>
          Register a web application with the provider, with this <b>redirect URI</b> (replace <code>&lt;id&gt;</code> with the entry's id):
        </p>
        <Code>{`${base}/api/auth/sso/<id>/callback`}</Code>
        <p>Take the client id and client secret it gives you. Then one entry per provider:</p>
        <Code>{`[
  { "type": "oidc", "id": "entra", "preset": "microsoft", "tenant": "<tenant id>",
    "clientId": "…", "clientSecret": "…", "syncGroups": true, "groupsClaim": "groups" },

  { "type": "oidc", "id": "google", "preset": "google",
    "clientId": "…", "clientSecret": "…", "allowedDomains": ["example.com"] },

  { "type": "oidc", "id": "keycloak", "name": "Company login",
    "issuer": "https://sso.example.com/realms/acme",
    "clientId": "agora", "clientSecret": "…" }
]`}</Code>
        <Options
          head={['Field', 'Meaning']}
          rows={[
            [<code>preset</code>, <><code>microsoft</code>, <code>google</code> or <code>gitlab</code>: the provider's addresses are known. Without a preset give <code>issuer</code>.</>],
            [<code>issuer</code>, 'The provider’s issuer address, for any other OpenID Connect provider (Okta, Keycloak, Authentik, …). Agora reads the rest from its discovery document.'],
            [<code>tenant</code>, 'Microsoft only, and required: your tenant’s id. Sign-in for any Microsoft account ("common") is not supported.'],
            [<><code>clientId</code>, <code>clientSecret</code></>, 'From the application you registered.'],
            [<code>scopes</code>, 'More scopes to ask for, if your provider needs them for the e-mail address or the groups.'],
            [<code>groupsClaim</code>, <>The claim that holds the person's groups (often <code>groups</code>; with Entra, <code>groups</code> or <code>roles</code>). Used with <code>syncGroups</code>.</>],
          ]}
        />
        <p style={{ marginBottom: 0 }}>
          For Google and Microsoft alone there are shortcuts without JSON: <code>AGORA_GOOGLE_CLIENT_ID</code>, <code>AGORA_GOOGLE_CLIENT_SECRET</code>, <code>AGORA_GOOGLE_ALLOWED_DOMAINS</code>, and <code>AGORA_MICROSOFT_CLIENT_ID</code>, <code>AGORA_MICROSOFT_CLIENT_SECRET</code>, <code>AGORA_MICROSOFT_TENANT</code>, <code>AGORA_MICROSOFT_ALLOWED_DOMAINS</code>.
        </p>
      </Section>

      <Section id="github" title="GitHub">
        <p>
          Create an OAuth app (GitHub → Settings → Developer settings → OAuth Apps) with the callback <code>{base}/api/auth/sso/&lt;id&gt;/callback</code>.
        </p>
        <Code>{`{ "type": "github", "id": "github", "clientId": "…", "clientSecret": "…", "allowedOrgs": ["acme"] }`}</Code>
        <p style={{ marginBottom: 0 }}>
          <code>allowedOrgs</code> admits only members of those organizations. For GitHub Enterprise Server add <code>apiUrl</code> and <code>webUrl</code>.
        </p>
      </Section>

      <Section id="ldap" title="LDAP and Active Directory">
        <p>People enter their directory user name and password on Agora's sign-in page; Agora checks them against the directory.</p>
        <Code>{`{ "type": "ldap", "id": "ad", "url": "ldaps://dc.example.com:636",
  "baseDn": "dc=example,dc=com",
  "bindDn": "cn=agora,ou=services,dc=example,dc=com", "bindPassword": "…",
  "syncGroups": true }`}</Code>
        <Options
          head={['Field', 'Meaning']}
          rows={[
            [<code>url</code>, <><code>ldaps://host:636</code>, or <code>ldap://host:389</code> with <code>"startTls": true</code>.</>],
            [<><code>bindDn</code>, <code>bindPassword</code></>, 'A service account that may search for users. Without it the search is anonymous.'],
            [<code>baseDn</code>, 'Where users are searched.'],
            [<code>userFilter</code>, <>How a user is found; <code>{'{username}'}</code> is replaced. By default uid, mail or sAMAccountName.</>],
            [<><code>emailAttribute</code>, <code>nameAttribute</code>, <code>idAttribute</code>, <code>groupAttribute</code></>, 'The attributes to read, when yours differ from the usual ones.'],
          ]}
        />
      </Section>

      <Section id="saml" title="SAML 2.0">
        <p>
          SAML needs a server built with it (the Docker image built with <code>--build-arg FEATURES=saml</code>). Give your identity provider these addresses:
        </p>
        <Options
          head={['', 'Address']}
          rows={[
            ['Assertion consumer service (ACS)', <code>{base}/api/auth/sso/&lt;id&gt;/acs</code>],
            ['Entity id and metadata of Agora', <code>{base}/api/auth/sso/&lt;id&gt;/metadata</code>],
          ]}
        />
        <Code>{`{ "type": "saml", "id": "corp", "name": "Company login",
  "idpMetadataUrl": "https://idp.example.com/metadata", "syncGroups": true }`}</Code>
        <p style={{ marginBottom: 0 }}>
          <code>idpMetadataXml</code> takes the metadata inline instead; <code>emailAttribute</code>, <code>nameAttribute</code> and <code>groupsAttribute</code> name the attributes when they differ.
        </p>
      </Section>

      <Section id="rules" title="Who gets in: domains, new accounts, groups">
        <p>These fields work on every kind of provider:</p>
        <Options
          head={['Field', 'Meaning']}
          rows={[
            [<code>allowedDomains</code>, 'Only e-mail addresses in these domains may sign in. The address must be one the provider vouches for (see trustEmail).'],
            [<code>autoCreate</code>, 'Create an account at a person’s first sign-in. On by default for your own directory (LDAP, SAML, a Microsoft tenant, your own issuer) and for public providers once allowedDomains or allowedOrgs restrict them; otherwise off, and only people who already have an account get in.'],
            [<code>defaultRole</code>, <>Role of accounts created this way: <code>member</code> (default) or <code>auditor</code>.</>],
            [<code>syncGroups</code>, 'Mirror the person’s groups from the provider into Agora’s groups at every sign-in. Model access, budgets and Desktop features can then follow your directory.'],
            [<code>trustEmail</code>, 'Accept e-mail addresses the provider doesn’t mark as verified. Already the case for LDAP, SAML and a single Microsoft tenant; off for other providers.'],
            [<code>name</code>, 'The text on the sign-in button.'],
          ]}
        />
        <ul>
          <li>A person who already has an account with the same verified e-mail address is linked to it at their first single sign-on.</li>
          <li>Group membership changes when the person signs in again, not before.</li>
          <li>Someone removed from your directory can no longer sign in, but their account, API tokens and signed-in desktop devices remain until you deactivate the account under <Link to="/admin/users">Users</Link>.</li>
          <li>There is no SCIM: accounts are created at sign-in, not pushed by the identity provider.</li>
        </ul>
      </Section>
    </>
  )
}

/** Groups, model access, budgets. */
export function Access() {
  return (
    <>
      <Section title="Groups">
        <p style={{ marginBottom: 0 }}>
          <Link to="/admin/groups">Groups</Link> are how you say who gets what. A group has members and, optionally, a monthly budget per member. Groups can come from your identity provider (see single sign-on) or be made here. A model, a Desktop feature and a local connector can each be open to everyone or to chosen groups.
        </p>
      </Section>
      <Section title="Who may use which model">
        <ul>
          <li>Each model under <Link to="/admin/models">Models</Link> is open to everyone or to selected groups. People only see the models they may use.</li>
          <li>A model can carry instructions of its own, a default effort and a description that people see in the model picker.</li>
          <li>The default model for new chats, and a model for writing chat titles, are under <Link to="/admin/settings">Settings</Link>.</li>
        </ul>
      </Section>
      <Section title="Budgets and limits">
        <Options
          head={['Limit', 'Where']}
          rows={[
            ['Monthly budget for one person (USD)', <>the person, under <Link to="/admin/users">Users</Link></>],
            ['Monthly budget per member of a group', <Link to="/admin/groups">Groups</Link>],
            ['Default monthly budget for everyone', <Link to="/admin/settings">Settings</Link>],
            ['Requests per minute, per person', <Link to="/admin/settings">Settings</Link>],
          ]}
        />
        <ul>
          <li>The most specific budget wins: the person's own, else the highest of their groups, else the default.</li>
          <li>It is checked before each message. When the month's cost reaches it, the next message is refused and says why.</li>
          <li>Cost comes from the prices on each model. Claude's list prices are filled in; set your own for other models, or leave them at zero for hardware of your own.</li>
          <li><Link to="/admin/usage">Usage</Link> shows cost and tokens per day, person, model and group, with a CSV export.</li>
        </ul>
      </Section>
      <Section title="What people are told">
        <p style={{ marginBottom: 0 }}>
          Under <Link to="/admin/settings">Settings</Link>: instructions that every model gets, a notice shown under the message box (for example what must not be entered), and whether people may give a chat instructions of their own.
        </p>
      </Section>
    </>
  )
}

/** Web search and knowledge sources. */
export function Sources() {
  return (
    <>
      <Section title="Web search">
        <p>Set up under <Link to="/admin/settings">Settings</Link>. With it, people get a Web search toggle, and the model decides when to search and which pages to read.</p>
        <Options
          head={['Engine', 'What you need']}
          rows={[
            ['SearXNG', 'The address of your own SearXNG instance. Nothing is sent to a search company by Agora.'],
            ['Brave Search API', 'An API key'],
            ['Serper (Google results)', 'An API key from serper.dev'],
            ['Tavily', 'An API key'],
            ['Claude’s own web search', 'Nothing more: Claude models search at Anthropic. Can be used instead of an engine for those models.'],
          ]}
        />
        <ul>
          <li><b>Reading pages</b> can be switched off, leaving search results only.</li>
          <li>A <b>block list</b> of domains applies to searches and page reads.</li>
          <li>Addresses in private networks are never opened, also not through redirects, unless the server is started with <code>AGORA_FETCH_ALLOW_PRIVATE=true</code>.</li>
          <li>Every search and page read is recorded under <Link to="/admin/web">Web activity</Link>: who, in which chat, the query or address, the results.</li>
        </ul>
      </Section>
      <Section title="Knowledge sources (WikiCraft)">
        <ol>
          <li><Link to="/admin/settings">Settings</Link> → Knowledge sources → add the wiki's address and a line on what is in it.</li>
          <li>Everyone connects it once for themselves under Settings &amp; usage, with a read-only API token they create in the wiki.</li>
          <li>The message box then has a toggle with the wiki's name. With it on, the model searches the wiki and cites the pages.</li>
        </ol>
        <p style={{ marginBottom: 0 }}>
          Because each person uses their own token, the wiki's permissions apply to every search: a model never sees a page its user couldn't open. For search by meaning and not only by words, set up embeddings in the wiki.
        </p>
      </Section>
    </>
  )
}

/** Audit and retention. */
export function Audit() {
  return (
    <>
      <Section title="What is recorded">
        <ul>
          <li>Sign-ins and failed sign-ins; every message and answer; web searches and page reads; knowledge searches; API calls; tool calls on people's computers; every administrative change.</li>
          <li>For messages and answers you choose under <Link to="/admin/settings">Settings</Link> between the full text and metadata only (model, size, tokens, cost).</li>
          <li><Link to="/admin/audit">Audit log</Link> has filters and exports (CSV, JSONL).</li>
        </ul>
      </Section>
      <Section title="Why it can be trusted">
        <p>
          Each event's hash covers its content and the hash of the event before it, and the database refuses to change or delete events. A gap or a changed event breaks the chain. The Administration overview checks it and says whether it is intact; from a terminal:
        </p>
        <Code>{'agora verify-audit        # exits with an error when the chain is broken'}</Code>
      </Section>
      <Section title="Auditors and retention">
        <ul>
          <li>The <b>auditor</b> role reads the audit log, usage and conversations and changes nothing. Opening someone's conversation is itself an audit event.</li>
          <li>A chat its owner deletes disappears for them and stays available to auditors for the retention period you set.</li>
          <li>Retention of chats and of audit events is set under <Link to="/admin/settings">Settings</Link>.</li>
        </ul>
      </Section>
    </>
  )
}

/** Agora Desktop, for administrators. */
export function DesktopAdmin() {
  return (
    <>
      <p className="muted">
        Agora Desktop lets a model act on a person's computer. Nothing of it is available until you grant it, under <Link to="/admin/desktop">Desktop</Link>.
      </p>
      <Section title="What you grant">
        <Options
          head={['Feature', 'What it decides']}
          rows={[
            ['Agora Desktop', 'Who may sign in with the app at all (nobody at first). Tool calls per answer, seconds a tool may take, sub-agents, slash commands.'],
            ['Code in dev containers', 'Who gets file and command tools. The container images projects may use, the default image, whether containers reach the network, whether the person confirms commands and file changes, how long a command may run.'],
            ['Browser', 'Who gets the browser tools. Allowed and blocked sites. Whether the person confirms clicks and typing.'],
            ['Local connectors', 'Programs the app may start, or addresses it may reach, on the computer (Model Context Protocol servers). Which of their tools models get, whether calls are confirmed, and for which groups.'],
          ]}
        />
        <p style={{ marginBottom: 0 }}>Each feature goes to nobody, to everyone or to chosen groups. A change reaches connected apps at once, and a tool that was withdrawn is no longer run, even in an answer that is being written.</p>
      </Section>
      <Section title="Slash commands">
        <p style={{ marginBottom: 0 }}>
          In the Agora Desktop card you define commands for everyone: a name, a line on what it does, and the prompt it stands for (<code>$ARGUMENTS</code> is what the person types after the name). You can also turn off commands that people keep in files of their own.
        </p>
      </Section>
      <Section title="Devices and what happened on them">
        <ul>
          <li><b>Devices</b> lists every signed-in computer with its owner; you can sign any of them out. People see and sign out their own under Settings &amp; usage.</li>
          <li><b>Tool activity</b> lists every call a model made on someone's computer: who, which computer, which tool with which input, how it ended. The same goes into the audit log.</li>
        </ul>
      </Section>
      <Section title="What is enforced where">
        <ul>
          <li><b>The server</b> offers a model only the tools its user was granted, only for messages sent from that person's app, and refuses blocked sites before a call leaves.</li>
          <li><b>The app</b> checks each call against the same rules again, asks the person where you want a confirmation, and keeps to the allowed images and sites on the computer. A changed app could skip the questions on its own computer.</li>
          <li>A connector's settings are sent to the apps of the people it is granted to: don't put secrets in them.</li>
        </ul>
      </Section>
      <Section title="Known weaknesses">
        <ul>
          <li>A device's sign-in is a full sign-in for its owner. Sign out lost computers under Devices.</li>
          <li>The browser is driven through a local debugging port that other programs on the computer can reach. Don't grant code with network access and the browser together to people who open untrusted projects.</li>
          <li>A project's own setup command (<code>postCreateCommand</code>) runs when its container is created, without a confirmation.</li>
          <li>Sites are checked after a page has loaded; a redirect can reach a blocked site for a moment.</li>
        </ul>
      </Section>
    </>
  )
}

/** Running the server. */
export function Server() {
  return (
    <>
      <Section title="Starting it">
        <p>With Docker:</p>
        <Code>{`export AGORA_DB_PASSWORD=$(openssl rand -hex 16) AGORA_SECRET_KEY=$(openssl rand -hex 32)
docker compose up -d
# the first visit creates the owner account`}</Code>
        <p style={{ marginBottom: 0 }}>
          Agora is one program and a PostgreSQL database. Everything that changes while it runs (models, groups, budgets, web search, retention, Desktop grants) is set in these admin pages. What follows is set when the server starts, as environment variables or flags.
        </p>
      </Section>
      <Section title="Server settings">
        <Options
          head={['Variable', 'Meaning (default)']}
          rows={[
            [<code>AGORA_DATABASE_URL</code>, 'The PostgreSQL database'],
            [<code>AGORA_BIND</code>, 'Address to listen on (127.0.0.1:8790)'],
            [<code>AGORA_BASE_URL</code>, 'The address people use; single sign-on redirects are built from it'],
            [<code>AGORA_SECRET_KEY</code>, 'Encrypts provider keys and tokens in the database. Set it, keep it safe, never change it: stored keys become unreadable otherwise'],
            [<code>AGORA_SECURE_COOKIES</code>, 'Set to true behind HTTPS (false)'],
            [<code>AGORA_TRUST_PROXY</code>, 'Take people’s addresses from X-Forwarded-For, behind a reverse proxy (false)'],
            [<><code>AGORA_SESSION_DAYS</code>, <code>AGORA_SESSION_IDLE_MINUTES</code></>, 'How long a sign-in lasts (14 days), and after how long without activity it ends (never)'],
            [<><code>AGORA_AUTH_PROVIDERS</code>, <code>AGORA_AUTH_PROVIDERS_FILE</code></>, 'Single sign-on providers as JSON'],
            [<code>AGORA_DISABLE_PASSWORD_LOGIN</code>, 'Single sign-on only (false)'],
            [<code>AGORA_FETCH_ALLOW_PRIVATE</code>, 'Let models open addresses in private networks (false)'],
            [<code>AGORA_MODEL_TIMEOUT_SECS</code>, 'How long to wait for a model to start answering, and between pieces of an answer (300)'],
            [<><code>AGORA_CLAUDE_BIN</code>, <code>AGORA_CLAUDE_DIR</code></>, 'Claude Code, for models that answer through it, and the folder it runs in (claude, ./data/claude-code)'],
            [<code>AGORA_LOG_FORMAT</code>, 'text, or json for log collectors'],
          ]}
        />
      </Section>
      <Section title="Behind a reverse proxy">
        <ul>
          <li>Set <code>AGORA_BASE_URL</code> to the public address, <code>AGORA_SECURE_COOKIES=true</code> and <code>AGORA_TRUST_PROXY=true</code>.</li>
          <li>Answers stream and the desktop app keeps a WebSocket open (<code>/api/desktop/ws</code>): the proxy must not buffer responses and must pass WebSocket upgrades, with timeouts long enough for a long answer.</li>
          <li>Messages can carry files: allow request bodies of 32 MB.</li>
        </ul>
      </Section>
      <Section title="Commands">
        <Code>{`agora serve            # the server (default)
agora migrate          # apply database changes and exit
agora verify-audit     # check the audit log's chain
agora healthcheck      # ask the running server whether it is well (for containers)`}</Code>
      </Section>
      <Section title="Backups and updates">
        <ul>
          <li><b>Back up the database and the secret key.</b> The database holds everything: accounts, chats, attached files, usage, the audit log, settings. Without the same <code>AGORA_SECRET_KEY</code>, the provider keys in a restored database can't be read.</li>
          <li>Sessions of models that answer through Claude Code are files in the server account's home folder. Losing them loses nothing visible: such a chat continues with a new session that is given the conversation.</li>
          <li><b>Updating:</b> start the new version. Database changes are applied when the server starts.</li>
          <li>With several servers behind a load balancer, a desktop app is connected to one of them, and its tools only work for answers written by that one.</li>
        </ul>
      </Section>
    </>
  )
}

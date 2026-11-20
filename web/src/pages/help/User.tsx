import { Link } from 'react-router-dom'
import type { Me } from '../../api'
import { Code, Options, Section, serverAddress } from './parts'

/** Chatting: what everyone uses. */
export function Chatting({ me }: { me: Me }) {
  return (
    <>
      <Section title="A chat">
        <ul>
          <li><b>New chat</b> at the top left starts one. Pick the model above the conversation; you can change it between messages.</li>
          <li><b>Enter</b> sends, <b>Shift+Enter</b> starts a new line. <b>Stop</b> ends an answer that is being written.</li>
          <li>An answer keeps being written when you close the tab. Open the chat again and it continues where it is.</li>
          <li>Under every answer you see the model, the tokens, the cost and the time it took.</li>
          <li>The list on the left has search; a chat can be renamed, pinned to the top or deleted from its menu. Titles are written automatically after the first answer.</li>
        </ul>
      </Section>
      <Section title="Changing your mind: edits and branches">
        <ul>
          <li><b>Edit</b> under one of your messages sends a changed version. The earlier version and its answers stay: flip between them with the arrows (‹ 1 / 2 ›).</li>
          <li><b>Regenerate</b> under an answer asks again, with the same or another model. That is a branch too.</li>
        </ul>
      </Section>
      <Section title="Projects">
        <p>
          A <Link to="/projects">project</Link> keeps chats together that are about the same thing, and gives the model the background once instead of in every chat.
        </p>
        <ul>
          {me.allowUserSystemPrompt && <li><b>Instructions:</b> what the model should know and do in every chat of the project.</li>}
          <li><b>Files:</b> text files of any kind (Markdown, CSV, code), images and PDFs. The model gets all of them with every message in the project, so large files make every answer cost more. A project holds up to 400,000 characters of text and 20 MB of images and PDFs.</li>
          <li><b>Chats:</b> start one with <b>New chat in this project</b> on the project's page. The folder icon above a conversation moves a chat into a project or out of it; the project applies from the next message on.</li>
          <li><b>Sharing:</b> a project is yours alone until you share it with everyone or with groups you are in. The others can read its instructions and files and start chats of their own in it. Nobody sees another person's chats, and only you can change the project.</li>
          <li>Deleting a project removes its instructions and files. Its chats stay, without the project.</li>
        </ul>
      </Section>
      <Section title="What a message can carry">
        <ul>
          <li><b>Images and PDFs:</b> the Attach button, or drop or paste them into the message box. Up to 8 files, images up to 5 MB, 20 MB per message. The button is off for models that can't read them.</li>
          {me.webSearch && <li><b>Web search:</b> with the toggle on, the model decides when to search and which pages to open, and cites them. Searches and page reads are recorded.</li>}
          <li><b>Knowledge sources:</b> if your organization connected a wiki, connect it for yourself once under <Link to="/settings">Settings &amp; usage</Link> with a read-only token of your own. A toggle with the wiki's name then lets the model search it with your permissions.</li>
          <li><b>Effort:</b> for models that think before answering, how hard they think. More effort gives better answers to hard questions and costs more.</li>
          {me.allowUserSystemPrompt && <li><b>Instructions for a chat:</b> the sliders icon above the conversation holds instructions that apply to that chat only.</li>}
        </ul>
      </Section>
      <Section title="Your usage and settings">
        <p>
          <Link to="/settings">Settings &amp; usage</Link> shows what you used this month and your budget if one is set, the light or dark theme, the computers signed in with Agora Desktop, your API tokens and your knowledge sources.
        </p>
        <p style={{ marginBottom: 0 }}>
          When a monthly budget is used up, new messages are refused until the next month or until an administrator raises it.
          {me.notice ? ' Your organization shows a notice under the message box: it says what may and may not be entered here.' : ''}
        </p>
      </Section>
      <Section title="What your organization can see">
        <p style={{ marginBottom: 0 }}>
          Agora keeps a record of sign-ins, messages and answers (their text or only their size, as your organization chose), web searches, and tool calls on your computer. Administrators see usage and costs per person; auditors can open conversations, and opening one is itself recorded. A chat you delete disappears for you; your organization may keep it for a retention period.
        </p>
      </Section>
    </>
  )
}

/** Agora Desktop, for the person using it. */
export function DesktopApp({ me }: { me: Me }) {
  return (
    <>
      <p className="muted">
        Agora Desktop is an app for your computer that signs in to this server. It gives a model what only a program on your computer can: your project's code in a dev container, a browser window, and local apps through connectors.
        {me.desktop ? '' : ' Your account isn’t granted the app at the moment: ask an administrator.'}
      </p>
      <Section title="Signing in">
        <ol>
          <li>Start the app and enter this server's address: <code>{serverAddress()}</code>.</li>
          <li>The app shows a code and opens this web app. Check that the code and the computer's name match, and approve. Single sign-on works as usual, because you sign in here, not in the app.</li>
          <li>The computer then appears under <Link to="/settings">Settings &amp; usage → Devices</Link>, where you can sign it out again.</li>
        </ol>
      </Section>
      <Section title="Working on code">
        <ul>
          <li><b>Open a project folder</b> in the panel on the right and switch <b>Code</b> on under the message box. The model reads, searches and edits files and runs commands inside a container that only has that folder. Docker (or Podman) must be installed.</li>
          <li>The container comes from the project's <code>.devcontainer/devcontainer.json</code> (its image, environment, user and setup command), otherwise from your organization's default image. It is kept between sessions.</li>
          <li><b>Confirmations:</b> depending on your organization's settings you confirm each command and file change: Allow, Allow in this chat, or Decline. You see the command or the change before you decide.</li>
          <li><b>Undo:</b> "Undo file changes" under an answer puts the files it wrote or edited back as they were. What commands changed is not covered: use git for that.</li>
          <li><b>Instructions for the model:</b> <code>AGENTS.md</code> (or <code>CLAUDE.md</code>) at the project root is given to the model with every message. Your own instructions for every project go in <code>instructions.md</code> in the app's settings folder.</li>
          <li><b>Notes that last:</b> the model keeps what is worth knowing next time in <code>.agora/memory.md</code> in the project. It is an ordinary file: read it, correct it, commit it or ignore it.</li>
          <li><b>Projects:</b> the list above the chats chooses a project. Its chats are shown, new chats go into it, and the folder you had open with it is opened again when you choose it. <b>New project…</b> in that list makes one, with its instructions and files; sharing it and changing it later happens here in the web app.</li>
          <li><b>Long commands:</b> the model gets a command's output while it runs and can start servers and watchers in the background. You see the output in the chat as it is printed.</li>
        </ul>
      </Section>
      <Section title="Browser and local connectors">
        <ul>
          <li><b>Browser:</b> the model opens pages, clicks and types in a browser window of its own, with a separate profile: not your everyday browser, its cookies or passwords. Password fields are refused; type those yourself in the window.</li>
          <li><b>Connectors:</b> programs on your computer that give the model access to local apps and data (Model Context Protocol servers). Your organization defines which exist; you switch them on per message.</li>
        </ul>
      </Section>
      <Section title="Slash commands">
        <p>Type <code>/</code> in the app's message box. What follows the name is given to the command.</p>
        <Options
          head={['Command', 'What it does']}
          rows={[
            [<code>/help</code>, 'Lists every command you can use'],
            [<code>/new</code>, 'Starts a new chat'],
            [<code>/compact</code>, 'Summarizes the conversation so far, to free the model’s context in a long session'],
            [<code>/undo</code>, 'Puts back the files the last answer wrote or edited'],
            [<code>/model name</code>, 'Switches the model'],
            [<code>/init</code>, 'Has the model write AGENTS.md for the open project'],
            ['Your organization’s', 'Commands your administrators defined; /help shows them'],
            ['Your own', <>One Markdown file per command in a project's <code>.agora/commands/</code> (or <code>.claude/commands/</code>), or in <code>commands/</code> in the app's settings folder. <code>$ARGUMENTS</code> in the file stands for what you type after the name. Your organization can turn these off.</>],
          ]}
        />
      </Section>
      <Section title="Without a window">
        <p>The <code>agora-agent</code> program does the same from a terminal, for scripts and build machines:</p>
        <Code>{`agora-agent login ${serverAddress()}
agora-agent ask --workspace ~/work/project --yes "Run the tests and fix what fails"
agora-agent ask --attach error.png "What does this dialog mean?"
agora-agent ask --project "Website relaunch" "Draft the launch mail"
agora-agent undo --workspace ~/work/project
agora-agent serve          # stay connected for chats started elsewhere`}</Code>
        <p style={{ marginBottom: 0 }}>Without <code>--yes</code>, anything that needs your confirmation is declined.</p>
      </Section>
    </>
  )
}

/** The OpenAI-compatible API. */
export function Api() {
  const base = serverAddress()
  return (
    <>
      <p className="muted">Other programs (an editor, a script, a tool that speaks the OpenAI API) can use the models you may use here. The same access rules, budgets and records apply.</p>
      <Section title="A token">
        <p style={{ marginBottom: 0 }}>
          Create one under <Link to="/settings">Settings &amp; usage → API tokens</Link>. It is shown once and starts with <code>agt_</code>. It acts as you: keep it like a password, and delete it there when it is no longer needed.
        </p>
      </Section>
      <Section title="Using it">
        <p>Point any OpenAI client at this address, with the token as its key:</p>
        <Options
          head={['Setting', 'Value']}
          rows={[
            ['Base URL', <code>{base}/api/v1</code>],
            ['API key', <code>agt_…</code>],
            ['Models', <>the names <code>GET {base}/api/v1/models</code> lists</>],
          ]}
        />
        <Code>{`curl ${base}/api/v1/chat/completions \\
  -H "Authorization: Bearer agt_…" -H "Content-Type: application/json" \\
  -d '{"model": "<a model name>", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'`}</Code>
        <p style={{ marginBottom: 0 }}>
          Chat completions with and without streaming are supported. Web search, knowledge sources and the tools of Agora Desktop belong to chats in Agora and are not part of this API. Models that answer through Claude Code are not available here.
        </p>
      </Section>
    </>
  )
}

import { useRef, useState, type JSX } from 'react';
import { ArrowRight, Shield, Terminal, GitBranch, ExternalLink, Github, Mail, Play, Pause } from 'lucide-react';

type Locale = 'en' | 'fr';

interface StepCopy {
  num: string;
  title: string;
  desc: string;
}

interface UseCaseCopy {
  title: string;
  desc: string;
}

interface SecurityBullet {
  text: string;
  tone: 'ready' | 'partial' | 'planned';
}

interface LocaleCopy {
  nav: { features: string; install: string; security: string; demo: string; github: string; contact: string };
  hero: { title: string; subtitle: string; points: string[]; cta: string; badge: string; disclaimer: string };
  how: { title: string; steps: StepCopy[] };
  usecases: { title: string; cases: UseCaseCopy[] };
  security: { title: string; subtitle: string; bullets: SecurityBullet[] };
  install: {
    title: string;
    prereqs: string;
    linux: string;
    windows: string;
    linuxSteps: string[];
    windowsSteps: string[];
  };
  mcp: { title: string; desc: string; steps: string[]; warning: string };
  demo: { title: string; note: string };
  support: { title: string; issues: string; email: string };
  footer: { copyright: string; status: string; note?: string };
}

// Centralized version strings (override with VITE_APP_VERSION at build-time)
const RAW_VERSION: string = (import.meta as any)?.env?.VITE_APP_VERSION || '0.1.0';
const VERSION_BADGE_EN = `v${RAW_VERSION} Preview`;
const VERSION_BADGE_FR = `v${RAW_VERSION} Aperçu`;
const FOOTER_STATUS_EN = `${VERSION_BADGE_EN} • APIs subject to rapid change`;
const FOOTER_STATUS_FR = `${VERSION_BADGE_FR} • APIs susceptibles d'évoluer rapidement`;

const COPY: Record<Locale, LocaleCopy> = {
  en: {
    nav: { features: 'Features', install: 'Install', security: 'Security', demo: 'Demo', github: 'GitHub', contact: 'Contact' },
    hero: {
      title: 'DevIT: coordinate humans & LLMs on real repos',
      subtitle: 'Sandbox-first tooling to let agents work without breaking your project.',
      points: [
        'CLI + daemon core with optional MCP bridge for Claude Desktop and other clients',
        'Apply patches, inspect files, and orchestrate tasks with approval workflows',
        'Linux is production-ready; macOS and Windows support are actively in progress'
      ],
      cta: 'Install the preview',
      badge: VERSION_BADGE_EN,
      disclaimer: 'Preview build — APIs and security defaults may change before 1.0.'
    },
    how: {
      title: 'How DevIT fits into your workflow',
      steps: [
        { num: '1', title: 'Install toolchain', desc: 'Build the CLI, daemon, and MCP server binaries via cargo install or cargo build.' },
        { num: '2', title: 'Secure the link', desc: 'Generate a shared secret and point CLI + MCP clients to the daemon socket.' },
        { num: '3', title: 'Delegate safely', desc: 'Your LLM uses tools such as devit_patch_apply or devit_exec while policies enforce guardrails.' }
      ]
    },
    usecases: {
      title: 'What you can do today',
      cases: [
        { title: 'Patch with guardrails', desc: 'Diff parser with rollback and policy downgrades for binaries, exec bits, or protected paths.' },
        { title: 'Keep repos clean', desc: 'Run tests and scripts in sandbox profiles so temporary files never leak into your tree.' },
        { title: 'Trace every action', desc: 'Append-only JSONL journal ready for alerting or dashboards, optional HMAC signatures available.' }
      ]
    },
    security: {
      title: 'Security posture (alpha)',
      subtitle: 'Defense-in-depth building blocks, with clear notes on what remains.',
      bullets: [
        { text: 'Path canonicalisation & traversal checks on every file operation', tone: 'ready' },
        { text: 'Approval engine with Ask / Moderate / Trusted tiers and automatic downgrades', tone: 'ready' },
        { text: 'Shared-secret HMAC signing between CLI ↔ daemon ↔ MCP transport', tone: 'ready' },
        { text: 'Replay protection (nonce tracking & timestamp window)', tone: 'planned' },
        { text: 'Sandbox runtimes (bubblewrap on Linux, Job Objects on Windows)', tone: 'partial' },
        { text: 'TLS termination built-in', tone: 'planned' },
        { text: 'Third-party security review', tone: 'planned' }
      ]
    },
    install: {
      title: 'Installation',
      prereqs: 'Requirements',
      linux: 'Linux (primary) / macOS (community verified)',
      windows: 'Windows (preview)',
      linuxSteps: [
        '# Install from source',
        'git clone https://github.com/n-engine/devit.git',
        'cd devit',
        'cargo install --path crates/cli',
        'cargo install --path devitd',
        'cargo install --path crates/mcp-server',
        '',
        '# Configure shared secret & daemon socket',
        'export DEVIT_SECRET="$(openssl rand -hex 32)"',
        'devitd --socket /tmp/devitd.sock --secret "$DEVIT_SECRET"',
        '',
        '# Point CLI to daemon',
        'export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock',
        'devit doctor'
      ],
      windowsSteps: [
        '# Build (MSVC toolchain required)',
        'git clone https://github.com/n-engine/devit.git',
        'cd devit',
        'cargo build --release --target x86_64-pc-windows-msvc',
        '',
        '# Run daemon (PowerShell)',
        '$env:DEVIT_SECRET = "<your-secret>"',
        '.\\run.ps1 -Socket \\\\.\\pipe\\devitd',
        '',
        '# Configure CLI',
        '$env:DEVIT_DAEMON_SOCKET = "\\\\.\\pipe\\devitd"',
        '.\\target\\x86_64-pc-windows-msvc\\release\\devit.exe doctor'
      ]
    },
    mcp: {
      title: 'Expose DevIT via MCP',
      desc: 'Serve DevIT tools to Claude Desktop or any MCP-compatible client.',
      steps: [
        'Run `mcp-server --transport http --host 0.0.0.0 --port 3001 --auth-token <token>`',
        'Host `/.well-known/mcp.json` with transport URLs (append `?ngrok-skip-browser-warning=1` when tunnelling)',
        'Reverse-proxy through Caddy or nginx for TLS; disable compression on `/sse`',
        'Register the manifest URL in your MCP client and call `tools/list` to verify'
      ],
      warning: 'Keep the MCP endpoint behind TLS and authentication; the built-in server does not enable HTTPS by itself.'
    },
    demo: {
      title: 'See DevIT in Action',
      note: "Watch Claude use DevIT's tools."
    },
    support: { title: 'Support', issues: 'GitHub Issues', email: 'Email' },
    footer: {
      copyright: '© 2025 DevIT.',
      status: FOOTER_STATUS_EN,
      note: 'Linux-first. Track OS parity in PROJECT_TRACKING/.'
    }
  },
  fr: {
    nav: { features: 'Fonctionnalités', install: 'Installation', security: 'Sécurité', demo: 'Démo', github: 'GitHub', contact: 'Contact' },
    hero: {
      title: 'DevIT : coordonnez humain et LLM sur votre dépôt',
      subtitle: 'Sandbox et approbations pour laisser un agent écrire du code en confiance.',
      points: [
        'Noyau CLI + daemon avec passerelle MCP optionnelle pour Claude Desktop et autres clients',
        'Application de patchs, inspection fichiers et orchestration avec validations automatiques',
        'Linux prêt à l\'emploi, macOS et Windows en cours de finalisation'
      ],
      cta: 'Installer l\'aperçu',
      badge: VERSION_BADGE_FR,
      disclaimer: 'Version d\'aperçu — les API et réglages sécurité peuvent évoluer avant la 1.0.'
    },
    how: {
      title: 'Comment DevIT s\'insère dans votre flux',
      steps: [
        { num: '1', title: 'Installer la toolchain', desc: 'Compilez le CLI, le daemon et le serveur MCP via cargo install ou cargo build.' },
        { num: '2', title: 'Sécuriser le lien', desc: 'Générez un secret partagé et reliez CLI + clients MCP au socket du daemon.' },
        { num: '3', title: 'Déléguer en sécurité', desc: 'Laissez le LLM utiliser devit_patch_apply ou devit_exec tandis que les politiques appliquent les garde-fous.' }
      ]
    },
    usecases: {
      title: 'Ce qui fonctionne aujourd\'hui',
      cases: [
        { title: 'Patch sous contrôle', desc: 'Parseur de diffs avec rollback et downgrade automatique pour binaires, exec bit ou chemins protégés.' },
        { title: 'Dépôt immaculé', desc: 'Tests et scripts dans des sandboxes temporaires pour éviter toute pollution du repo.' },
        { title: 'Traçabilité fine', desc: 'Journal JSONL append-only, signatures HMAC optionnelles pour alertes ou dashboards.' }
      ]
    },
    security: {
      title: 'Sécurité (alpha)',
      subtitle: 'Briques de défense déjà en place, avec transparence sur la suite.',
      bullets: [
        { text: 'Canonicalisation des chemins et protection contre la traversée', tone: 'ready' },
        { text: 'Moteur d\'approbation Ask / Moderate / Trusted avec dégradations automatiques', tone: 'ready' },
        { text: 'Signature HMAC entre CLI ↔ daemon ↔ transport MCP', tone: 'ready' },
        { text: 'Protection replay (nonce + fenêtre temporelle)', tone: 'planned' },
        { text: 'Runtimes sandbox (bubblewrap sur Linux, Job Objects sur Windows)', tone: 'partial' },
        { text: 'Terminaison TLS intégrée', tone: 'planned' },
        { text: 'Audit sécurité externe', tone: 'planned' }
      ]
    },
    install: {
      title: 'Installation',
      prereqs: 'Prérequis',
      linux: 'Linux (prioritaire) / macOS (validé par la communauté)',
      windows: 'Windows (aperçu)',
      linuxSteps: [
        '# Installation depuis les sources',
        'git clone https://github.com/n-engine/devit.git',
        'cd devit',
        'cargo install --path crates/cli',
        'cargo install --path devitd',
        'cargo install --path crates/mcp-server',
        '',
        '# Secret partagé + socket daemon',
        'export DEVIT_SECRET="$(openssl rand -hex 32)"',
        'devitd --socket /tmp/devitd.sock --secret "$DEVIT_SECRET"',
        '',
        '# Configurer le CLI',
        'export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock',
        'devit doctor'
      ],
      windowsSteps: [
        '# Compilation (toolchain MSVC)',
        'git clone https://github.com/n-engine/devit.git',
        'cd devit',
        'cargo build --release --target x86_64-pc-windows-msvc',
        '',
        '# Lancer le daemon (PowerShell)',
        '$env:DEVIT_SECRET = "<votre-secret>"',
        '.\\run.ps1 -Socket \\\\.\\pipe\\devitd',
        '',
        '# Configurer le CLI',
        '$env:DEVIT_DAEMON_SOCKET = "\\\\.\\pipe\\devitd"',
        '.\\target\\x86_64-pc-windows-msvc\\release\\devit.exe doctor'
      ]
    },
    mcp: {
      title: 'Exposez DevIT via MCP',
      desc: 'Servez les outils DevIT à Claude Desktop ou tout client compatible MCP.',
      steps: [
        'Exécutez `mcp-server --transport http --host 0.0.0.0 --port 3001 --auth-token <token>`',
        'Servez `/.well-known/mcp.json` avec les URLs de transport (ajoutez `?ngrok-skip-browser-warning=1` avec ngrok)',
        'Placez le tout derrière Caddy/nginx pour le TLS et désactivez la compression sur `/sse`',
        'Enregistrez l\'URL du manifest dans votre client MCP puis testez `tools/list`'
      ],
      warning: 'Laissez toujours MCP derrière TLS + authentification ; le serveur natif ne gère pas HTTPS nativement.'
    },
    demo: {
      title: 'DevIT en action',
      note: "Regardez Claude orchestrer l\'implémentation complète d'un jeu Tetris avec les outils de délégation et monitoring de DevIT."
    },
    support: { title: 'Support', issues: 'Tickets GitHub', email: 'Email' },
    footer: {
      copyright: '© 2025 DevIT.',
      status: FOOTER_STATUS_FR,
      note: 'Priorité Linux. Parité OS suivie dans PROJECT_TRACKING/.'
    }
  }
};

const SECURITY_BADGE_TONE: Record<SecurityBullet['tone'], string> = {
  ready: 'bg-emerald-900/40 border-emerald-500/30 text-emerald-200',
  partial: 'bg-amber-900/30 border-amber-500/40 text-amber-200',
  planned: 'bg-slate-900/40 border-slate-600/40 text-slate-300'
};

export default function App(): JSX.Element {
  const [locale, setLocale] = useState<Locale>('en');
  const [isPlaying, setIsPlaying] = useState(true);
  const installRef = useRef<HTMLElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const copy = COPY[locale];

  const scrollToInstall = () => installRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' });

  const toggleVideo = () => {
    if (videoRef.current) {
      if (isPlaying) {
        videoRef.current.pause();
      } else {
        videoRef.current.play();
      }
      setIsPlaying(!isPlaying);
    }
  };

  return (
    <div className="min-h-screen bg-slate-950 text-white">
      <header className="sticky top-0 z-30 border-b border-slate-800/70 bg-slate-950/70 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
          <a className="flex items-center gap-3 font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#top">
            <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-gradient-to-br from-blue-500 to-cyan-500 text-lg font-bold">
              D
            </span>
            <span className="text-xl">DevIT</span>
          </a>
          <nav className="hidden items-center gap-6 text-sm md:flex" aria-label="Main navigation">
            <a className="transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#features">
              {copy.nav.features}
            </a>
            <a className="transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#install">
              {copy.nav.install}
            </a>
            <a className="transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#security">
              {copy.nav.security}
            </a>
            <a className="transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#demo">
              {copy.nav.demo}
            </a>
            <a
              className="flex items-center gap-1 transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
              href="https://github.com/n-engine/devit"
              target="_blank"
              rel="noopener noreferrer"
            >
              {copy.nav.github}
              <ExternalLink size={14} aria-hidden="true" />
            </a>
            <a className="transition hover:text-cyan-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400" href="#contact">
              {copy.nav.contact}
            </a>
          </nav>
          <button
            type="button"
            className="rounded border border-slate-700 px-3 py-1 text-sm transition hover:bg-slate-800 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
            onClick={() => setLocale((prev) => (prev === 'en' ? 'fr' : 'en'))}
          >
            {locale === 'en' ? 'FR' : 'EN'}
          </button>
        </div>
      </header>

      <main id="top" className="relative overflow-hidden">
        <div className="pointer-events-none absolute inset-0">
          <div className="absolute left-10 top-20 h-72 w-72 rounded-full bg-blue-500/20 blur-3xl" />
          <div className="absolute bottom-20 right-0 h-72 w-72 rounded-full bg-cyan-500/20 blur-3xl" />
        </div>

        <section className="relative mx-auto grid max-w-6xl gap-12 px-6 py-24 md:grid-cols-2 md:py-32" aria-labelledby="hero-title">
          <div className="space-y-8">
            <span className="inline-flex items-center gap-2 rounded-full border border-orange-500/40 bg-orange-900/30 px-4 py-1 text-sm text-orange-200">
              🟠 {copy.hero.badge}
            </span>
            <div className="space-y-5">
              <h1 id="hero-title" className="text-5xl font-black md:text-6xl">
                <span className="bg-gradient-to-r from-blue-400 via-cyan-300 to-blue-400 bg-clip-text text-transparent">{copy.hero.title}</span>
              </h1>
              <p className="text-lg text-cyan-200">{copy.hero.subtitle}</p>
              <ul className="space-y-2 text-slate-300">
                {copy.hero.points.map((point) => (
                  <li key={point} className="flex gap-2">
                    <span aria-hidden="true">•</span>
                    <span>{point}</span>
                  </li>
                ))}
              </ul>
            </div>
            <div className="flex flex-col gap-3">
              <button
                type="button"
                onClick={scrollToInstall}
                className="group flex w-fit items-center gap-2 rounded-lg bg-gradient-to-r from-blue-500 to-cyan-500 px-7 py-3 font-semibold text-slate-950 transition hover:shadow-lg hover:shadow-blue-500/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
              >
                {copy.hero.cta}
                <ArrowRight size={18} className="transition group-hover:translate-x-1" aria-hidden="true" />
              </button>
              <p className="text-sm text-slate-400">{copy.hero.disclaimer}</p>
            </div>
          </div>

          <aside className="relative hidden h-full md:block" aria-hidden="true">
            <div className="absolute inset-0 rounded-2xl bg-gradient-to-r from-slate-800/60 to-slate-900/60 blur-3xl" />
            <div className="relative rounded-2xl border border-slate-700/70 bg-slate-900/80 p-6 font-mono text-xs text-slate-300 shadow-xl backdrop-blur">
              <span className="text-slate-500">~/workspace/devit</span>
              <pre className="mt-3 space-y-2">
                <code className="block text-cyan-300">$ devit snapshot --pretty</code>
                <code className="block text-emerald-400">✓ snapshot created</code>
                <code className="block text-cyan-300">$ devit patch-apply fix.diff --dry-run</code>
                <code className="block text-emerald-400">✓ policy check: Ask → Moderate downgrade</code>
                <code className="block text-emerald-400">✓ journal entry #418 appended</code>
              </pre>
            </div>
          </aside>
        </section>

        <section id="features" className="relative mx-auto max-w-6xl px-6 py-20" aria-labelledby="features-title">
          <h2 id="features-title" className="text-center text-4xl font-bold">
            {copy.how.title}
          </h2>
          <div className="mt-12 grid gap-6 md:grid-cols-3">
            {copy.how.steps.map((step) => (
              <article key={step.num} className="rounded-xl border border-slate-700/60 bg-slate-900/70 p-8 text-center shadow">
                <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-gradient-to-br from-blue-400 to-cyan-400 text-lg font-bold">
                  {step.num}
                </div>
                <h3 className="text-lg font-semibold text-cyan-200">{step.title}</h3>
                <p className="mt-2 text-sm text-slate-300">{step.desc}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="relative mx-auto max-w-6xl px-6 py-20" aria-labelledby="usecases-title">
          <h2 id="usecases-title" className="text-center text-4xl font-bold">
            {copy.usecases.title}
          </h2>
          <div className="mt-12 grid gap-6 md:grid-cols-3">
            {copy.usecases.cases.map((usecase) => (
              <article key={usecase.title} className="rounded-xl border border-slate-700/60 bg-slate-900/70 p-6 shadow transition hover:border-cyan-500/50">
                <h3 className="text-lg font-semibold text-cyan-200">{usecase.title}</h3>
                <p className="mt-3 text-sm text-slate-300">{usecase.desc}</p>
              </article>
            ))}
          </div>
        </section>

        <section id="security" className="relative mx-auto max-w-6xl px-6 py-20" aria-labelledby="security-title">
          <h2 id="security-title" className="text-center text-4xl font-bold">
            {copy.security.title}
          </h2>
          <p className="mt-4 text-center text-slate-300">{copy.security.subtitle}</p>
          <div className="mt-12 grid gap-6 md:grid-cols-2">
            <ul className="space-y-4">
              {copy.security.bullets.map((bullet) => (
                <li key={bullet.text} className="flex items-start gap-3 rounded-lg border border-slate-700/50 bg-slate-900/70 p-4">
                  <Shield className="mt-0.5 h-5 w-5 text-cyan-300" aria-hidden="true" />
                  <div>
                    <p className="text-sm text-slate-200">{bullet.text}</p>
                    <span className={`mt-2 inline-flex rounded-full px-3 py-1 text-xs font-semibold ${SECURITY_BADGE_TONE[bullet.tone]}`}>
                      {bullet.tone === 'ready' && 'Available'}
                      {bullet.tone === 'partial' && 'Partial'}
                      {bullet.tone === 'planned' && 'Planned'}
                    </span>
                  </div>
                </li>
              ))}
            </ul>
            <div className="space-y-4 rounded-xl border border-slate-700/60 bg-slate-900/70 p-6">
              <div className="flex items-center gap-3 text-slate-200">
                <Terminal className="h-5 w-5 text-cyan-300" aria-hidden="true" />
                <span>CLI ↔ daemon authentication enforced when a shared secret is configured.</span>
              </div>
              <div className="flex items-center gap-3 text-slate-200">
                <GitBranch className="h-5 w-5 text-cyan-300" aria-hidden="true" />
                <span>Journal tracks operation metadata for audits and agent feedback.</span>
              </div>
              <div className="flex items-center gap-3 text-slate-200">
                <Shield className="h-5 w-5 text-cyan-300" aria-hidden="true" />
                <span>Policy engine downgrades or blocks actions touching protected paths or exec bits.</span>
              </div>
            </div>
          </div>
        </section>

        <section id="install" ref={installRef} className="relative mx-auto max-w-6xl px-6 py-20" aria-labelledby="install-title">
          <h2 id="install-title" className="text-center text-4xl font-bold">
            {copy.install.title}
          </h2>
          <div className="mt-12 space-y-6">
            <div className="rounded-xl border border-slate-700/60 bg-slate-900/70 p-6">
              <p className="text-sm font-semibold text-slate-300">{copy.install.prereqs}:</p>
              <ul className="mt-3 space-y-1 text-sm text-slate-300">
                <li>• Rust 1.79+, Git, OpenSSL (Linux/macOS)</li>
                <li>• Visual Studio Build Tools & Rust MSVC toolchain (Windows)</li>
              </ul>
            </div>
            <div className="grid gap-6 md:grid-cols-2">
              <article className="rounded-xl border border-slate-700/60 bg-slate-900/70 p-6">
                <h3 className="text-lg font-semibold text-cyan-200">{copy.install.linux}</h3>
                <pre className="mt-4 whitespace-pre-wrap break-words text-xs text-slate-300">{copy.install.linuxSteps.join('\n')}</pre>
              </article>
              <article className="rounded-xl border border-slate-700/60 bg-slate-900/70 p-6">
                <h3 className="text-lg font-semibold text-cyan-200">{copy.install.windows}</h3>
                <pre className="mt-4 whitespace-pre-wrap break-words text-xs text-slate-300">{copy.install.windowsSteps.join('\n')}</pre>
              </article>
            </div>
            <section className="rounded-xl border border-blue-700/40 bg-blue-900/20 p-6">
              <h3 className="text-lg font-bold text-cyan-200">{copy.mcp.title}</h3>
              <p className="mt-2 text-sm text-slate-300">{copy.mcp.desc}</p>
              <ol className="mt-4 list-inside list-decimal space-y-2 text-sm text-slate-200">
                {copy.mcp.steps.map((step) => (
                  <li key={step}>{step}</li>
                ))}
              </ol>
              <p className="mt-4 text-xs text-amber-200">{copy.mcp.warning}</p>
            </section>
          </div>
        </section>

        <section id="demo" className="relative mx-auto max-w-6xl px-6 py-20" aria-labelledby="demo-title">
          <div className="space-y-8">
            <div className="text-center">
              <h2 id="demo-title" className="text-4xl font-bold">
                {copy.demo.title}
              </h2>
              <p className="mt-4 text-lg text-slate-300">{copy.demo.note}</p>
            </div>

            <div className="group relative overflow-hidden rounded-2xl border border-slate-700/60 bg-slate-900/70 shadow-2xl">
              {/* Gradient overlay */}
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950/80 via-transparent to-transparent opacity-60" />
              
              {/* Video */}
              <video
                ref={videoRef}
                className="w-full"
                autoPlay
                loop
                muted
                playsInline
                onClick={toggleVideo}
              >
                <source src="/devit-demo.mp4" type="video/mp4" />
                Your browser does not support the video tag.
              </video>

              {/* Play/Pause overlay button */}
              <button
                type="button"
                onClick={toggleVideo}
                className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-slate-900/80 p-6 backdrop-blur transition-all hover:scale-110 hover:bg-slate-800/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
                aria-label={isPlaying ? 'Pause video' : 'Play video'}
              >
                {isPlaying ? (
                  <Pause size={32} className="text-cyan-400" aria-hidden="true" />
                ) : (
                  <Play size={32} className="ml-1 text-cyan-400" aria-hidden="true" />
                )}
              </button>

              {/* Bottom gradient bar */}
              <div className="absolute bottom-0 left-0 right-0 h-1 bg-gradient-to-r from-blue-500 via-cyan-500 to-blue-500" />
            </div>
          </div>
        </section>

        <section id="contact" className="relative mx-auto max-w-4xl px-6 pb-28" aria-labelledby="support-title">
          <h2 id="support-title" className="text-center text-3xl font-bold">
            {copy.support.title}
          </h2>
          <div className="mt-10 grid gap-6 md:grid-cols-2">
            <a
              href="https://github.com/n-engine/devit/issues"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-4 rounded-xl border border-slate-700/60 bg-slate-900/70 p-6 transition hover:border-cyan-500/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
            >
              <Github className="h-6 w-6 text-cyan-300" aria-hidden="true" />
              <div>
                <div className="font-semibold text-slate-100">{copy.support.issues}</div>
                <div className="text-xs text-slate-400">github.com/n-engine/devit</div>
              </div>
            </a>
            <a
              href="mailto:contact@getdevit.com"
              className="flex items-center gap-4 rounded-xl border border-slate-700/60 bg-slate-900/70 p-6 transition hover:border-cyan-500/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
            >
              <Mail className="h-6 w-6 text-cyan-300" aria-hidden="true" />
              <div>
                <div className="font-semibold text-slate-100">{copy.support.email}</div>
                <div className="text-xs text-slate-400">contact@getdevit.com</div>
              </div>
            </a>
          </div>
        </section>
      </main>

      <footer className="border-t border-slate-800/70 bg-slate-950/90">
        <div className="mx-auto max-w-6xl px-6 py-12 text-center text-sm text-slate-400">
          <p>{copy.footer.copyright}</p>
          <p className="mt-1 text-xs text-slate-500">
            {copy.footer.status}
            {copy.footer.note ? ` • ${copy.footer.note}` : ''}
          </p>
        </div>
      </footer>
    </div>
  );
}

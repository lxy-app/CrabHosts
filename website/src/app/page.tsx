import { DesktopPreview } from '@/components/desktop-preview';
import { ThemeToggle } from '@/components/theme-toggle';

export default function Home() {
  return (
    <>
      <header><div className="wrap bar">
        <a className="brand" href="/crab-hosts/"><img src="/crab-hosts/assets/icon.png" alt="" />CrabHosts</a>
        <nav aria-label="主导航"><a href="#preview">产品预览</a><a href="#cli">命令行</a><a href="https://github.com/lxy-app/CrabHosts">GitHub ↗</a></nav>
        <ThemeToggle />
      </div></header>
      <main>
      <section className="hero wrap">
        <div className="intro"><p className="eyebrow"><i></i> LOCAL HOSTS MANAGER</p>
          <h1>CrabHosts</h1><h2>分环境管理本机 hosts</h2>
          <p className="lead">把开发、测试和生产环境的域名映射分别存入 Profile。切换前逐行查看差异，确认后再写入系统。</p>
          <div className="actions"><a className="primary" href="https://github.com/lxy-app/CrabHosts#构建与运行">开始使用 <span>↗</span></a><a className="secondary" href="#preview">查看产品预览 <span>↓</span></a></div>
          <p className="stack">Rust <b>·</b> Slint <b>·</b> CLI</p>
        </div>
        <div className="terminal" aria-label="hosts 配置示例"><div className="terminal-head"><div className="dots" aria-hidden="true"><i></i><i></i><i></i></div><span>dev.hosts</span></div>
          <pre><code><em># 本地开发环境</em>{'\n\n'}<span>127.0.0.1</span>    api.local{'\n'}<span>127.0.0.1</span>    admin.local{'\n'}<span>::1</span>          localhost</code></pre>
          <div className="status"><i></i> Profile <strong>dev</strong></div>
        </div>
      </section>
      <DesktopPreview />
      <section className="cli wrap" id="cli"><div><p className="eyebrow">COMMAND LINE</p><h2>从终端预览变更</h2><p>桌面端和 CLI 共用 Profile。选取已有配置，通过 dry-run 检查待写入的内容。自动化工具也可以读取 JSON 结果。</p><a href="https://github.com/lxy-app/CrabHosts/blob/main/skills/crabhosts/references/cli.md">阅读 CLI 文档 ↗</a></div><pre><code><em># 列出本机配置</em>{'\n'}crabhosts --json profile list{'\n\n'}<em># 预览名为 dev 的配置</em>{'\n'}crabhosts system apply dev --dry-run</code></pre></section>
      </main><footer><div className="wrap bar"><span>CrabHosts</span><a href="https://github.com/lxy-app/CrabHosts">GitHub ↗</a></div></footer>
    </>
  );
}

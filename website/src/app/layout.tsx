import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import './globals.css';

export const metadata: Metadata = {
  title: 'CrabHosts · 本地 hosts 管理工具',
  description: '用 CrabHosts 分环境管理本机 hosts，预览差异、确认应用并自动备份。桌面端与 CLI 共享同一套 Profile。',
  alternates: { canonical: 'https://lxy.app/crab-hosts/' },
  icons: { icon: '/crab-hosts/assets/icon.png' },
};

const initialTheme = `try {
  const theme = localStorage.getItem('crabhosts.theme');
  if (theme === 'dark' || theme === 'light') document.documentElement.dataset.theme = theme;
} catch {}`;

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="zh-CN" suppressHydrationWarning>
      <head><script dangerouslySetInnerHTML={{ __html: initialTheme }} /></head>
      <body>{children}</body>
    </html>
  );
}

# CrabHosts 主页

主页基于 Next.js 16 App Router、React 19 和 TypeScript。页面使用静态导出，主题切换和桌面预览在浏览器中运行。

在本目录使用 Node.js 24 安装依赖、启动开发服务：

```sh
npm ci
npm run dev
```

打开 http://127.0.0.1:5174/crab-hosts/ 。初次访问跟随系统外观，手动选择的主题会在刷新后保留。

```sh
npm run typecheck
npm run build
```

将生成的 `out/` 中全部内容发布到 `https://lxy.app/crab-hosts/` 对应的静态目录，包括 `_next/` 和 `assets/`。主机应将 `/crab-hosts` 重定向到 `/crab-hosts/` 并提供目录首页。

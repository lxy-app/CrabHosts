'use client';

import { useState } from 'react';

const previews = [
  { title: '配置管理', image: 'preview.png', detail: '按环境整理 IP 和域名，单独启用或停用每条记录。' },
  { title: '差异预览', image: 'preview-diff-fullscreen.png', detail: '逐行比较当前系统内容和待应用的变更。' },
  { title: '应用确认', image: 'preview-apply.png', detail: '确认计划后应用配置，写入前自动备份当前 hosts。' },
] as const;

export function DesktopPreview() {
  const [active, setActive] = useState<(typeof previews)[number]>(previews[0]);
  return (
    <section className="showcase wrap" id="preview">
      <div className="section-head">
        <div><p className="eyebrow">DESKTOP</p><h2>桌面工作区</h2></div>
        <div className="choices" aria-label="选择界面预览" role="group">
          {previews.map(preview => (
            <button key={preview.image} type="button" aria-pressed={active === preview}
              onClick={() => setActive(preview)}>{preview.title}</button>
          ))}
        </div>
      </div>
      <figure>
        <div className="stage">
          <img id="screen" src={'/crab-hosts/assets/' + active.image} alt={'CrabHosts ' + active.title + '界面'} />
        </div>
        <figcaption aria-live="polite"><strong>{active.title}</strong> {active.detail}</figcaption>
      </figure>
    </section>
  );
}

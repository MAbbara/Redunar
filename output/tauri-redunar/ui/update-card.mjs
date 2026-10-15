// Presentation only: the native updater owns verification, installation and restart.
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

export function updateIcon(kind='download') {
 const paths={download:'M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4',restart:'M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1',check:'M5 12l4 4L19 6',warning:'M12 8v5m0 4h.01M12 3 2 21h20Z',spinner:'M20 12a8 8 0 1 1-8-8'};
 return `<svg class="updater-icon ${kind==='spinner'?'updater-spinner':''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${paths[kind]||paths.download}"/></svg>`;
}

export function updateSidebar(status,{version,pending}) {
 const state=status?.state||'unchecked';
 const kind=['installing','restarting'].includes(pending)?'spinner':state==='available'?'download':state==='restart-needed'?'restart':null;
 const label=pending==='installing'?'Installing update':pending==='restarting'?'Restarting Redunar':state==='available'?'Update available':state==='restart-needed'?'Restart needed':'';
 return `${kind?`<span class="sidebar-update-badge" role="img" aria-label="${escape(label)}" title="${escape(label)}">${updateIcon(kind)}</span>`:''}<span class="sidebar-version-number">v${escape(version)}</span>`;
}

export function updateCard(status, {native, busy, restartBlocked, pending}) {
 const state=pending||status?.state||'unchecked';
 const release=status?.latest_version?`Redunar ${status.latest_version}`:'The update';
 const copy={
  unchecked:['Check for updates','Find the latest Redunar release.'],
  'up-to-date':['You’re up to date',status?.message||'The latest Redunar version is installed.'],
  available:['Update available',status?.message||`${release} is ready to install.`],
  'restart-needed':['Update installed',status?.latest_version?`Restart to use ${release}.`:'Restart Redunar to use the installed update.'],
  handoff:['Installation not confirmed',status?.message||'Check again or retry installation.'],
  error:['Update error',status?.message||'Try checking again.'],
  'not-configured':['Updates unavailable',status?.message||'No update source is configured.'],
  checking:['Checking for updates…','Looking for the latest release.'],
  installing:['Installing update…','Your system may ask for authorization.'],
  restarting:['Restarting Redunar…','Opening the installed version.'],
 };
 const [title,description]=copy[state]||['Update status unavailable',status?.message||'Check again for the latest status.'];
 const disabled=!native||busy||Boolean(pending);
 const action=(label,name,blocked=disabled)=>`<button type="button" class="button primary" data-action="${name}" ${blocked?'disabled':''}>${label}</button>`;
 const refresh=`<button type="button" class="update-refresh" data-action="check-for-updates" title="Check for updates" aria-label="Check for updates" ${disabled?'disabled':''}>${updateIcon('restart')}</button>`;
 let actions;
 if(state==='restart-needed'||state==='restarting')actions=action('Restart Redunar','restart-redunar',disabled||restartBlocked);
 else if(['available','handoff','installing'].includes(state))actions=refresh+action(state==='handoff'?'Retry update':'Update now','install-update');
 else actions=action(state==='error'?'Try again':'Check for updates','check-for-updates');
 const version=['handoff','restart-needed'].includes(state)&&status?.latest_version?`<span class="update-release-version">v${escape(status.latest_version)}</span>`:'';
 const explanation=['error','handoff','not-configured'].includes(state)||(!pending&&status?.operation_issue)?description:'';
 const glyph=pending?'spinner':state==='restart-needed'?'restart':['error','handoff','not-configured'].includes(state)?'warning':'download';
 return `<div class="update-status-card" data-state="${escape(state)}" aria-busy="${Boolean(pending)}"><div class="update-emblem">${updateIcon(glyph)}</div><div class="update-status-copy"><div class="update-status-heading" role="status"><strong data-update-state>${escape(title)}</strong>${version}</div><p class="update-status-description" ${explanation?'':'hidden'} ${state==='error'?'role="alert"':''}>${escape(explanation)}</p></div><div class="update-card-actions">${actions}</div></div>`;
}

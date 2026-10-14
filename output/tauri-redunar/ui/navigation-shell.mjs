// Presentation only: this does not change session, profile or window ownership.
const sidebarPreferenceKey = 'redunar-sidebar-collapsed';
const toggle = document.querySelector('[data-sidebar-toggle]');
let collapsed = false;
try { collapsed = localStorage.getItem(sidebarPreferenceKey) === 'true'; } catch { /* Storage may be unavailable; navigation still works. */ }
function applySidebarPreference() {
  document.documentElement.dataset.sidebar = collapsed ? 'collapsed' : 'expanded';
  if (!toggle) return;
  const label = collapsed ? 'Expand sidebar' : 'Collapse sidebar';
  toggle.setAttribute('aria-expanded', String(!collapsed));
  toggle.setAttribute('aria-label', label);
  toggle.title = label;
  toggle.querySelector('span').textContent = label;
}
applySidebarPreference();
toggle?.addEventListener('click', () => {
  collapsed = !collapsed;
  applySidebarPreference();
  try { localStorage.setItem(sidebarPreferenceKey, String(collapsed)); } catch { /* Retain the current choice until this window closes. */ }
});

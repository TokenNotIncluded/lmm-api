(() => {
  const pages = /* PAGE_CATALOG */ {};
  const origin = /* PUBLIC_ORIGIN */ "";
  const sync = () => {
    const path = location.pathname === '/' ? '/' : location.pathname.replace(/\/$/, '');
    const page = pages[path];
    const title = page ? `${page[0]} · LMM Forge` : 'Account and workspace · LMM Forge';
    if (document.title !== title) document.title = title;
    const meta = (attribute, name, value) => {
      let element = document.head.querySelector(`meta[${attribute}="${name}"]`);
      if (value === null) { element?.remove(); return; }
      if (!element) { element = document.createElement('meta'); element.setAttribute(attribute, name); document.head.append(element); }
      if (element.content !== value) element.content = value;
    };
    meta('name', 'robots', page ? 'index, follow' : 'noindex, follow');
    meta('name', 'description', page ? page[1] : 'Sign in to manage your LMM Forge account and workspace.');
    let canonical = document.head.querySelector('link[rel="canonical"]');
    if (page) {
      if (!canonical) { canonical = document.createElement('link'); canonical.rel = 'canonical'; document.head.append(canonical); }
      if (canonical.href !== origin + path) canonical.href = origin + path;
    } else canonical?.remove();
    for (const [key, value] of Object.entries({type:'website', title, description:page?.[1], url:origin + path})) meta('property', `og:${key}`, page ? value : null);
  };
  for (const method of ['pushState', 'replaceState']) {
    const original = history[method];
    history[method] = function (...args) { const result = Reflect.apply(original, this, args); sync(); return result; };
  }
  addEventListener('popstate', sync);
  // The installed app starts only when its React root is empty. Keep the
  // crawlable fallback outside that root and remove it once React mounts.
  const fallback = document.getElementById('lmm-seo-content');
  const root = document.getElementById('root');
  if (fallback && root) {
    const mounted = () => { if (root.hasChildNodes()) { fallback.remove(); observer.disconnect(); } };
    const observer = new MutationObserver(mounted);
    observer.observe(root, {childList:true});
    mounted();
  }
  sync();
  new MutationObserver(sync).observe(document.head, {subtree:true, childList:true, characterData:true, attributes:true, attributeFilter:['content', 'href']});
})();

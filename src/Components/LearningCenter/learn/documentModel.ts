import { parse, type DefaultTreeAdapterTypes } from 'parse5';
import { safeCourseLink } from './markdownModel';

type Node = DefaultTreeAdapterTypes.Node;
export type DocumentNode = {
  tag: string;
  text?: string;
  id?: string;
  url?: string;
  fragment?: string;
  children: DocumentNode[];
};
export type DocumentView = { nodes: DocumentNode[]; error?: string } | null;
const blocked = new Set([
  'script',
  'style',
  'noscript',
  'template',
  'iframe',
  'frame',
  'frameset',
  'object',
  'embed',
  'svg',
  'math',
  'form',
  'input',
  'button',
  'textarea',
  'select',
  'option',
  'base',
  'link',
  'meta',
  'head',
  'nav',
  'aside',
  'footer',
  'audio',
  'video',
  'source',
]);
const allowed = new Set([
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'p',
  'div',
  'span',
  'section',
  'article',
  'ul',
  'ol',
  'li',
  'pre',
  'code',
  'strong',
  'b',
  'em',
  'i',
  'u',
  's',
  'small',
  'br',
  'hr',
  'blockquote',
  'table',
  'thead',
  'tbody',
  'tfoot',
  'tr',
  'td',
  'th',
  'caption',
  'dl',
  'dt',
  'dd',
  'sup',
  'sub',
  'figure',
  'figcaption',
  'details',
  'summary',
]);
const attr = (node: DefaultTreeAdapterTypes.Element, name: string) =>
  node.attrs.find((a) => a.name === name)?.value;

export function documentUrl(value: string, sourceUrl?: string): string | null {
  if (!value.trim()) return null;
  try {
    const base = sourceUrl ? safeCourseLink(sourceUrl) : null;
    const url = new URL(value, base || undefined);
    const host = url.hostname.toLowerCase();
    if (
      /^(?:127\.|10\.|0\.|169\.254\.|192\.168\.|172\.(?:1[6-9]|2\d|3[01])\.)/.test(host) ||
      host.endsWith('.localhost') ||
      host.endsWith('.local') ||
      host.endsWith('.internal') ||
      /^\[(?:::|f[cd]|fe[89ab]|::ffff:)/.test(host)
    )
      return null;
    return safeCourseLink(url.href);
  } catch {
    return null;
  }
}

/** Parse into data only: no browser HTML insertion, DOM fetches, scripts or source attributes. */
export function documentView(source: string, sourceUrl?: string): DocumentView {
  // An indented HTML example is Markdown code, not a whole HTML snapshot.
  if (/^(?: {4}|\t)/.test(source)) return null;
  if (
    !/^\s*(?:<!doctype\s+html\b|<(?:html|body|main|article|section|div|h[1-6]|p|table|ul|ol|nav|header|footer|pre|blockquote|figure|dl|details)\b)/i.test(
      source
    )
  )
    return null;
  if (source.length > 1024 * 1024) return { nodes: [], error: '教材正文过大，暂时无法显示。' };
  try {
    const tree = parse(source);
    let count = 0;
    const elements: DefaultTreeAdapterTypes.Element[] = [];
    function scan(node: Node, depth: number) {
      if (++count > 20000 || depth > 96) throw new Error('limit');
      if ('tagName' in node) {
        if (node.namespaceURI !== 'http://www.w3.org/1999/xhtml' || blocked.has(node.tagName))
          return;
        elements.push(node);
      }
      if ('childNodes' in node) node.childNodes.forEach((n) => scan(n, depth + 1));
    }
    scan(tree, 0);
    // Some source sanitizers retain anchors/IDs but turn main/nav/footer into divs.
    // Follow the source's explicit accessibility landmark; do not guess by course name.
    const nodeText = (node: Node): string =>
      'value' in node
        ? node.value
        : 'childNodes' in node
          ? node.childNodes.map(nodeText).join('')
          : '';
    const skipLink = elements.find(
      (n) =>
        n.tagName === 'a' &&
        attr(n, 'href')?.startsWith('#') &&
        /跳(?:到|转至).*(?:主要内容|正文)|skip\s+to\s+(?:main\s+)?content/i.test(nodeText(n))
    );
    let skipTarget = '';
    try {
      skipTarget = skipLink ? decodeURIComponent(attr(skipLink, 'href')?.slice(1) || '') : '';
    } catch {
      /* Missing/malformed landmarks do not discard content. */
    }
    const root =
      elements.find((n) => n.tagName === 'article') ||
      elements.find((n) => n.tagName === 'main' || attr(n, 'role') === 'main') ||
      elements.find((n) => skipTarget && attr(n, 'id') === skipTarget) ||
      elements.find((n) => n.tagName === 'body');
    let heading = 0;
    function convert(node: Node): DocumentNode[] {
      if (node.nodeName === '#text' && 'value' in node)
        return [{ tag: '#text', text: node.value, children: [] }];
      if (
        !('tagName' in node) ||
        blocked.has(node.tagName) ||
        node.namespaceURI !== 'http://www.w3.org/1999/xhtml'
      )
        return [];
      const children = node.childNodes.flatMap(convert);
      const id =
        attr(node, 'id') ||
        (/^h[1-6]$/.test(node.tagName) ? `learn-heading-${++heading}` : undefined);
      if (node.tagName === 'img') {
        const url = documentUrl(attr(node, 'src') || '', sourceUrl);
        return url
          ? [{ tag: 'img', url, text: attr(node, 'alt') || '教材插图', children: [] }]
          : [];
      }
      if (node.tagName === 'a') {
        const href = attr(node, 'href') || '';
        if (href.startsWith('#') && href.length > 1) {
          let fragment: string;
          try {
            fragment = decodeURIComponent(href.slice(1));
          } catch {
            return children;
          }
          return [{ tag: 'a', fragment, id, children }];
        }
        const url = documentUrl(href, sourceUrl);
        return url ? [{ tag: 'a', url, id, children }] : children;
      }
      return [{ tag: allowed.has(node.tagName) ? node.tagName : 'div', id, children }];
    }
    const nodes = root ? root.childNodes.flatMap(convert) : [];
    const visible = (node: DocumentNode): boolean =>
      Boolean(node.text?.trim()) || node.tag === 'img' || node.children.some(visible);
    return { nodes: nodes.some(visible) ? nodes : [] };
  } catch {
    return { nodes: [], error: '教材结构过于复杂，暂时无法显示。' };
  }
}

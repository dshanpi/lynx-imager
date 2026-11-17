export type MarkdownBlock = { kind: 'h1' | 'h2' | 'h3' | 'li' | 'p' | 'code'; text: string };
export function markdownBlocks(source: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  let fence = '';
  let code: string[] = [];
  for (const line of source.replace(/\r\n/g, '\n').split('\n')) {
    const opening = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (
        opening &&
        opening[1][0] === fence[0] &&
        opening[1].length >= fence.length &&
        !opening[2].trim()
      ) {
        blocks.push({ kind: 'code', text: code.join('\n') });
        code = [];
        fence = '';
      } else code.push(line);
      continue;
    }
    if (opening && (opening[1][0] !== '`' || !opening[2].includes('`'))) {
      fence = opening[1];
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({
        kind: heading[1].length === 1 ? 'h1' : heading[1].length === 2 ? 'h2' : 'h3',
        text: heading[2],
      });
      continue;
    }
    if (/^\s*[-*] /.test(line)) blocks.push({ kind: 'li', text: line.replace(/^\s*[-*] /, '') });
    else if (line.trim()) blocks.push({ kind: 'p', text: line });
  }
  if (fence) blocks.push({ kind: 'code', text: code.join('\n') });
  return blocks;
}
export function safeCourseLink(source: string): string | null {
  try {
    const u = new URL(source);
    if (
      u.protocol !== 'https:' ||
      u.username ||
      u.password ||
      u.hostname === 'localhost' ||
      u.hostname === '127.0.0.1' ||
      u.hostname === '[::1]'
    )
      return null;
    return u.href;
  } catch {
    return null;
  }
}

import type { FomodFileEntry } from './types'

// A node in the inferred virtual output tree. Directory nodes carry no size and
// hold children; file nodes are leaves with a size (and optional source path).
export interface TreeNode {
  name: string
  path: string
  size?: number
  source?: string
  isDir: boolean
  children: TreeNode[]
}

function sortNodes(nodes: TreeNode[]): void {
  nodes.sort((a, b) => {
    if (a.isDir !== b.isDir) {
      return a.isDir ? -1 : 1
    }
    return a.name.localeCompare(b.name)
  })
  for (const node of nodes) {
    if (node.isDir && node.children.length > 0) {
      sortNodes(node.children)
    }
  }
}

// Split each flat "a/b/c.dds" output entry into nested {name,path,...} nodes,
// folding shared prefixes into shared directory nodes. Directories sort before
// files, then alphabetically, so the tree is stable regardless of input order.
export function filesToTree(entries?: FomodFileEntry[]): TreeNode[] {
  const roots: TreeNode[] = []
  if (!entries || entries.length === 0) {
    return roots
  }

  const dirIndex = new Map<string, TreeNode>()

  for (const entry of entries) {
    if (!entry || typeof entry.path !== 'string') {
      continue
    }
    const parts = entry.path.split('/').filter(Boolean)
    if (parts.length === 0) {
      continue
    }

    let siblings = roots
    let prefix = ''
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      const isLeaf = i === parts.length - 1
      prefix = prefix ? `${prefix}/${part}` : part

      if (isLeaf) {
        siblings.push({
          name: part,
          path: prefix,
          size: entry.size,
          source: entry.source,
          isDir: false,
          children: [],
        })
        continue
      }

      let dir = dirIndex.get(prefix)
      if (!dir) {
        dir = { name: part, path: prefix, isDir: true, children: [] }
        dirIndex.set(prefix, dir)
        siblings.push(dir)
      }
      siblings = dir.children
    }
  }

  sortNodes(roots)
  return roots
}

// Depth-first flatten honouring a collapsed-folder set, so a virtualized list
// can render only the currently visible rows. Each row carries its depth for
// indentation.
export interface FlatNode {
  node: TreeNode
  depth: number
}

export function flattenTree(nodes: TreeNode[], collapsed: ReadonlySet<string>, depth = 0): FlatNode[] {
  const out: FlatNode[] = []
  for (const node of nodes) {
    out.push({ node, depth })
    if (node.isDir && node.children.length > 0 && !collapsed.has(node.path)) {
      out.push(...flattenTree(node.children, collapsed, depth + 1))
    }
  }
  return out
}

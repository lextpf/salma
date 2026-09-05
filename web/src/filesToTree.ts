import { FAULT_RANK, type FaultKind, type FomodFileEntry } from './types'

export interface TreeNode {
  name: string
  path: string
  size?: number
  source?: string
  isDir: boolean
  children: TreeNode[]
  fault?: FaultKind
  worstFault?: FaultKind
  absent?: boolean
}

function worseFault(a?: FaultKind, b?: FaultKind): FaultKind | undefined {
  if (!a) return b
  if (!b) return a
  return FAULT_RANK[a] <= FAULT_RANK[b] ? a : b
}

// propagate the worst descendant fault to each directory.
function rollUpFaults(nodes: TreeNode[]): FaultKind | undefined {
  let worst: FaultKind | undefined
  for (const node of nodes) {
    if (node.isDir) {
      node.worstFault = rollUpFaults(node.children)
      worst = worseFault(worst, node.worstFault)
    } else {
      worst = worseFault(worst, node.fault)
    }
  }
  return worst
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

/**
 * @fn filesToTree(entries?: FomodFileEntry[], faults?: ReadonlyMap<string, FaultKind>): TreeNode[]
 * @brief include installed-only faults in the simulated output hierarchy.
 * @author Alex (https://github.com/lextpf)
 *
 * missing installed paths become absent leaves. directories retain their worst
 * descendant fault for collapsed display.
 */
export function filesToTree(
  entries?: FomodFileEntry[],
  faults?: ReadonlyMap<string, FaultKind>,
): TreeNode[] {
  const roots: TreeNode[] = []
  const dirIndex = new Map<string, TreeNode>()
  const seen = new Set<string>()

  // return the leaf container, or null for an unusable path.
  const descend = (path: string): { siblings: TreeNode[]; leaf: string } | null => {
    const parts = path.split('/').filter(Boolean)
    if (parts.length === 0) {
      return null
    }
    let siblings = roots
    let prefix = ''
    for (let i = 0; i < parts.length - 1; i++) {
      prefix = prefix ? `${prefix}/${parts[i]}` : parts[i]
      let dir = dirIndex.get(prefix)
      if (!dir) {
        dir = { name: parts[i], path: prefix, isDir: true, children: [] }
        dirIndex.set(prefix, dir)
        siblings.push(dir)
      }
      siblings = dir.children
    }
    return { siblings, leaf: parts[parts.length - 1] }
  }

  for (const entry of entries ?? []) {
    if (!entry || typeof entry.path !== 'string') {
      continue
    }
    const spot = descend(entry.path)
    if (!spot) {
      continue
    }
    seen.add(entry.path)
    spot.siblings.push({
      name: spot.leaf,
      path: entry.path,
      size: entry.size,
      source: entry.source,
      isDir: false,
      children: [],
      fault: faults?.get(entry.path),
    })
  }

  if (faults) {
    for (const [path, kind] of faults) {
      if (kind !== 'missing' || seen.has(path)) {
        continue
      }
      const spot = descend(path)
      if (!spot) {
        continue
      }
      seen.add(path)
      spot.siblings.push({
        name: spot.leaf,
        path,
        isDir: false,
        children: [],
        fault: 'missing',
        absent: true,
      })
    }
  }

  sortNodes(roots)
  rollUpFaults(roots)
  return roots
}

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

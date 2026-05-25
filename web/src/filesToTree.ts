import { FAULT_RANK, type FaultKind, type FomodFileEntry } from './types'

// A node in the inferred virtual output tree. Directory nodes carry no size and
// hold children; file nodes are leaves with a size (and optional source path).
export interface TreeNode {
  name: string
  path: string
  size?: number
  source?: string
  isDir: boolean
  children: TreeNode[]
  /** How this file diverged from the installed mod, if it did. Files only. */
  fault?: FaultKind
  /**
   * The worst fault anywhere beneath a directory, so a collapsed folder still
   * shows that something inside it is wrong. Directories only.
   */
  worstFault?: FaultKind
  /**
   * True for a row the inferred selection does not produce: the file exists in
   * the installed mod and the simulation never wrote it. Such a row has no size
   * and no source, because no simulated file stands behind it.
   */
  absent?: boolean
}

/** The worse of two faults, either possibly undefined. */
function worseFault(a?: FaultKind, b?: FaultKind): FaultKind | undefined {
  if (!a) return b
  if (!b) return a
  return FAULT_RANK[a] <= FAULT_RANK[b] ? a : b
}

/**
 * Push each directory's worst descendant fault up the tree, so a fault stays
 * visible when the folder holding it is collapsed. Returns the subtree's worst.
 */
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

// Split each flat "a/b/c.dds" output entry into nested {name,path,...} nodes,
// folding shared prefixes into shared directory nodes. Directories sort before
// files, then alphabetically, so the tree is stable regardless of input order.
//
// `faults` marks the rows that diverged from the installed mod. Its `missing`
// entries are a special case: those files exist in the mod but the inferred
// selection never produces them, so they have no `outputTree` entry and are
// grafted in as `absent` leaves. Without that, the one fault a reader most
// wants to see would be the one with no row to look at.
export function filesToTree(
  entries?: FomodFileEntry[],
  faults?: ReadonlyMap<string, FaultKind>,
): TreeNode[] {
  const roots: TreeNode[] = []
  const dirIndex = new Map<string, TreeNode>()
  const seen = new Set<string>()

  // Walk a path down the tree, creating directories as needed, and return the
  // sibling list its leaf belongs in (or null if the path is unusable).
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

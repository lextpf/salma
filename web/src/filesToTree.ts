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

/**
 * @fn worseFault(a?: FaultKind, b?: FaultKind): FaultKind | undefined
 * @brief Keep the more severe fault when combining descendants.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param a First fault, if any.
 * @param b Second fault, if any.
 * @return The fault with the lower FAULT_RANK, or undefined when neither exists.
 */
function worseFault(a?: FaultKind, b?: FaultKind): FaultKind | undefined {
  if (!a) return b
  if (!b) return a
  return FAULT_RANK[a] <= FAULT_RANK[b] ? a : b
}

/**
 * @fn rollUpFaults(nodes: TreeNode[]): FaultKind | undefined
 * @brief Store the most severe descendant fault on each directory.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param nodes Subtree whose directory summaries are updated in place.
 * @return The most severe fault in the subtree, or undefined for a fault-free subtree.
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

/**
 * @fn sortNodes(nodes: TreeNode[]): void
 * @brief Sort each directory in place for stable browsing.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param nodes Subtree to sort, with directories before files and locale-based name order.
 */
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
 * @brief Include installed-only faults in the simulated output hierarchy.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Fault paths marked missing become absent leaves when they have no output entry.
 * Directories retain their worst descendant fault for collapsed display.
 * Paths must already share the engine's lowercase, slash-separated format; this function
 * does not normalize case or backslashes. Duplicate output entries remain separate leaves.
 *
 * @param entries Simulated output entries; an omitted list is empty.
 * @param faults Optional faults keyed by the same normalized path.
 * @return A new tree, sorted with directories first. Source entries are not modified.
 */
export function filesToTree(
  entries?: FomodFileEntry[],
  faults?: ReadonlyMap<string, FaultKind>,
): TreeNode[] {
  const roots: TreeNode[] = []
  const dirIndex = new Map<string, TreeNode>()
  const seen = new Set<string>()

  /**
   * @fn descend(path: string): { siblings: TreeNode[]; leaf: string } | null
   * @brief Reuse parent directories while locating the destination for a leaf.
   * @author Alex (<https://github.com/lextpf>)
   *
   * @param path Slash-separated path; empty components are ignored.
   * @return The leaf container and name, or null when the path has no components.
   */
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

/**
 * @fn flattenTree(nodes: TreeNode[], collapsed: ReadonlySet<string>, depth = 0): FlatNode[]
 * @brief Produce visible rows while retaining collapsed directory rows.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param nodes Tree in display order.
 * @param collapsed Directory paths whose children are hidden.
 * @param depth Initial nesting depth, normally zero.
 * @return Depth-first rows that reference the original nodes.
 */
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

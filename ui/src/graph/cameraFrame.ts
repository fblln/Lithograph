/**
 * Frames the complete bounded slice instead of assuming every repository fits
 * inside the original 12-unit demo camera. Real repositories routinely span
 * tens of world units once their directory groups are separated.
 */
export function cameraFrameForPositions(positions: Map<string, [number, number, number]>) {
  if (positions.size === 0) return { position: [6, 6, 6] as [number, number, number], target: [0, 0, 0] as [number, number, number], fov: 50 }
  let minX = Infinity
  let maxX = -Infinity
  let minY = Infinity
  let maxY = -Infinity
  let minZ = Infinity
  let maxZ = -Infinity
  for (const [x, y, z] of positions.values()) {
    minX = Math.min(minX, x)
    maxX = Math.max(maxX, x)
    minY = Math.min(minY, y)
    maxY = Math.max(maxY, y)
    minZ = Math.min(minZ, z)
    maxZ = Math.max(maxZ, z)
  }
  const centerX = (minX + maxX) / 2
  const centerY = (minY + maxY) / 2
  const centerZ = (minZ + maxZ) / 2
  const span = Math.max(maxX - minX, maxY - minY, maxZ - minZ, 4)
  const distance = span * 1.6
  return {
    // Cluster and matrix layouts live on the XZ plane. A near top-down
    // overview keeps every region at essentially the same camera distance;
    // the former diagonal view made foreground labels several times larger
    // than equally important clusters at the back of the repository.
    position: [centerX, centerY + distance, centerZ + distance * 0.035] as [number, number, number],
    target: [centerX, centerY, centerZ] as [number, number, number],
    fov: 50,
    near: Math.max(0.01, distance / 1000),
    far: Math.max(1000, distance * 20),
  }
}

/**
 * Frames the complete bounded slice instead of assuming every repository fits
 * inside the original 12-unit demo camera. Real repositories routinely span
 * tens of world units once their directory groups are separated.
 */
export function cameraFrameForPositions(
  positions: Map<string, [number, number, number]>,
  aspect = 16 / 10,
) {
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
  // Fit each ground-plane axis against its own viewport dimension instead of
  // framing the largest span with one flat multiplier. A repository layout is
  // usually much wider (X) than deep (Z); the old `max-span * 1.6` distance
  // backed the camera out until the WIDTH fit with a 60% margin, which left a
  // wide layout as a thin band in an empty canvas -- the live Flask smoke
  // measured cluster labels occupying 11% of the viewport span (minimum 16%).
  const halfFov = (50 / 2) * (Math.PI / 180)
  const spanX = Math.max(maxX - minX, 4)
  const spanZ = Math.max(maxZ - minZ, 4)
  const spanY = maxY - minY
  const distanceForWidth = spanX / 2 / (Math.tan(halfFov) * aspect)
  const distanceForDepth = spanZ / 2 / Math.tan(halfFov)
  // The Y spread is small for the planar layouts but must never clip.
  const distance = Math.max(distanceForWidth, distanceForDepth, spanY) * 1.25
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

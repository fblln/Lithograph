import { expect, test } from '@playwright/test'

interface Diagnostics {
  clusterLabelCount: number
  clippedClusterLabels: string[]
  clusterLabelScaleRatio: number
  nodeLabelScaleRatio: number
  overlappingNodeLabelRatio: number
  clusterSpreadRatio: number
  overlayOcclusionRatio: number
  issues: string[]
}

test('real graph remains readable in architecture and tension views', async ({ page }, testInfo) => {
  await page.goto('/?maxNodes=1000&maxEdges=1600&visualDiagnostics=1')
  await expect(page.getByText('Ready')).toBeVisible()
  await expect(page.locator('[data-visual-role="cluster-label"]').first()).toBeVisible()
  await assertReadable(page, testInfo, 'architecture')
  await page.getByRole('button', { name: 'Tensions' }).click()
  await expect(page.getByRole('heading', { name: 'Tension hotspots' })).toBeVisible()
  await assertReadable(page, testInfo, 'tensions')
})

async function assertReadable(page: import('@playwright/test').Page, testInfo: import('@playwright/test').TestInfo, phase: string) {
  await page.evaluate(async () => { await document.fonts.ready; await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))) })
  const diagnostics = await page.evaluate(() => {
    const probe = window.__LITHOGRAPH_VISUAL_DIAGNOSTICS__
    if (!probe) throw new Error('visual diagnostics were not installed; include ?visualDiagnostics=1')
    return probe.collect()
  }) as Diagnostics
  await testInfo.attach(`${phase}-diagnostics.json`, { body: JSON.stringify(diagnostics, null, 2), contentType: 'application/json' })
  await testInfo.attach(`${phase}.png`, { body: await page.screenshot(), contentType: 'image/png' })

  expect(diagnostics.clusterLabelCount, diagnostics.issues.join('\n')).toBeGreaterThan(0)
  expect(diagnostics.clippedClusterLabels, diagnostics.issues.join('\n')).toEqual([])
  expect(diagnostics.clusterLabelScaleRatio, diagnostics.issues.join('\n')).toBeLessThanOrEqual(1.35)
  expect(diagnostics.nodeLabelScaleRatio, diagnostics.issues.join('\n')).toBeLessThanOrEqual(1.35)
  expect(diagnostics.overlappingNodeLabelRatio, diagnostics.issues.join('\n')).toBeLessThanOrEqual(0.25)
  expect(diagnostics.clusterSpreadRatio, diagnostics.issues.join('\n')).toBeGreaterThanOrEqual(0.16)
  expect(diagnostics.overlayOcclusionRatio, diagnostics.issues.join('\n')).toBeLessThanOrEqual(0.18)
  expect(diagnostics.issues).toEqual([])
}

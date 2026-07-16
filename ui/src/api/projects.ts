import { callServerTool } from './rpc'

export interface ProjectMetadata {
  id: string
  name: string
  is_primary: boolean
}

/** Lists only safe display metadata for roots explicitly allowlisted at server start. */
export async function listProjects(signal?: AbortSignal): Promise<ProjectMetadata[]> {
  const value = await callServerTool<unknown>('list_projects', {}, signal)
  if (!Array.isArray(value)) throw new Error('malformed project list')
  const projects = value.filter((item): item is ProjectMetadata => {
    if (!item || typeof item !== 'object') return false
    const candidate = item as Partial<ProjectMetadata>
    return typeof candidate.id === 'string'
      && typeof candidate.name === 'string'
      && typeof candidate.is_primary === 'boolean'
  })
  if (projects.length === 0 || !projects.some((project) => project.is_primary)) {
    throw new Error('project list has no primary project')
  }
  return projects
}

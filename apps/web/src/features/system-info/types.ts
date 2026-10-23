/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
export type SystemInstanceStatus = 'online' | 'stale'

/** Version and resource facts for one split native service process. */
export type SystemProcessInfo = {
  version?: string
  pid?: number
  started_at?: number
  cpu_percent?: number
  memory_bytes?: number
  memory_percent?: number
  threads?: number
  [key: string]: unknown
}

export type SystemInstanceInfo = {
  schema_version?: number
  reporter?: {
    id?: string
    slot?: string
    [key: string]: unknown
  }
  node?: {
    name?: string
    source?: string
    manually_configured?: boolean
    should_configure_manually?: boolean
    [key: string]: unknown
  }
  role?: {
    is_master?: boolean
    [key: string]: unknown
  }
  runtime?: {
    version?: string
    goos?: string
    goarch?: string
    started_at?: number
    instance_slot?: string
    [key: string]: unknown
  }
  host?: {
    hostname?: string
    [key: string]: unknown
  }
  resources?: {
    cpu?: {
      usage_percent?: number
      [key: string]: unknown
    }
    memory?: {
      usage_percent?: number
      [key: string]: unknown
    }
    /** Host-wide sample, distinct from process-level samples below. */
    system?: {
      cpu?: {
        usage_percent?: number
        cores?: number
        load_1?: number
        load_5?: number
        load_15?: number
        [key: string]: unknown
      }
      memory?: {
        total_bytes?: number
        used_bytes?: number
        free_bytes?: number
        usage_percent?: number
        [key: string]: unknown
      }
      [key: string]: unknown
    }
    /** Core and Extensions process samples, keyed by service name. */
    processes?: {
      core?: SystemProcessInfo
      extensions?: SystemProcessInfo
      [key: string]: SystemProcessInfo | undefined
    }
    storage?: {
      total_bytes?: number
      used_bytes?: number
      free_bytes?: number
      used_percent?: number
      [key: string]: unknown
    }
    [key: string]: unknown
  }
  [key: string]: unknown
}

export type SystemResourceSnapshot = {
  schema_version: number
  observed_at_unix_ms: number
  sample_window_ms: number
  system: {
    logical_cpus: number
    load_average: {
      one: number
      five: number
      fifteen: number
    }
    cpu_usage_percent: number
    memory_total_bytes: number
    memory_available_bytes: number
    memory_used_bytes: number
    memory_usage_percent: number
  }
  processes: {
    core?: SystemProcessInfo
    extensions?: SystemProcessInfo
    [key: string]: SystemProcessInfo | undefined
  }
}

export type SystemInstance = {
  // node_name is the legacy reporter key; reporter_id is its explicit alias.
  node_name: string
  reporter_id?: string
  physical_node_name?: string
  instance_slot?: string
  status: SystemInstanceStatus
  stale_after_seconds: number
  started_at: number
  last_seen_at: number
  info?: SystemInstanceInfo
}

export type SystemInstanceListResponse = {
  success: boolean
  message: string
  data?: SystemInstance[]
}

export type SystemInstanceDeleteResponse = {
  success: boolean
  message: string
  data?: {
    deleted_count: number
  }
}

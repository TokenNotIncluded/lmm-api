/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

export interface DiscountCode {
  id: number
  code: string
  name: string
  discount_percent: number
  min_amount: number
  status: number
  used_count: number
  /** 0 means this administrator code has no usage cap. */
  max_uses: number
  created_by: number
  created_time: number
  updated_time: number
  starts_time: number
  expired_time: number
}

export interface DiscountCodePage {
  items: DiscountCode[]
  total: number
  page: number
  page_size: number
}

export interface DiscountCodeResponse<T = unknown> {
  success: boolean
  data?: T
  message?: string
}

export interface DiscountCodeInput {
  id?: number
  code: string
  name: string
  discount_percent: number
  min_amount: number
  max_uses: number
  starts_time: number
  expired_time: number
  status?: number
}

export interface DiscountCodeBatchInput {
  name: string
  count: number
  discount_percent: number
  min_amount: number
  max_uses: number
  starts_time: number
  expired_time: number
}

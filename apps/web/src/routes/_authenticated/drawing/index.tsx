/*
Copyright (C) 2026 LIghtJUNction
*/
import { createFileRoute } from '@tanstack/react-router'

import { Drawing } from '@/features/drawing'

export const Route = createFileRoute('/_authenticated/drawing/')({
  component: Drawing,
})

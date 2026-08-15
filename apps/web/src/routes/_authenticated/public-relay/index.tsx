/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import { createFileRoute } from '@tanstack/react-router'

import { PublicRelay } from '@/features/public-relay'

export const Route = createFileRoute('/_authenticated/public-relay/')({
  component: PublicRelay,
})

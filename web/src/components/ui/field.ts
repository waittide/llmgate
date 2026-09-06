import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

import Field from "./field.vue"
import FieldSet from "./field-set.vue"
import FieldLegend from "./field-legend.vue"
import FieldGroup from "./field-group.vue"
import FieldContent from "./field-content.vue"
import FieldLabel from "./field-label.vue"
import FieldTitle from "./field-title.vue"
import FieldDescription from "./field-description.vue"
import FieldSeparator from "./field-separator.vue"
import FieldError from "./field-error.vue"

export type FieldOrientation = "vertical" | "horizontal" | "responsive"

export interface FieldVariantProps {
  orientation?: FieldOrientation | null
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

// fieldVariants 生成字段布局变体类名。
export function fieldVariants(options?: FieldVariantProps): string {
  const base = "group/field flex w-full gap-3 data-[invalid=true]:text-destructive"
  const orientation = options?.orientation || "vertical"
  const map: Record<FieldOrientation, string> = {
    vertical: "flex-col [&>*]:w-full [&>.sr-only]:w-auto",
    horizontal:
      "flex-row items-center [&>[data-slot=field-label]]:flex-auto has-[>[data-slot=field-content]]:items-start has-[>[data-slot=field-content]]:[&>[role=checkbox],[role=radio]]:mt-px",
    responsive:
      "flex-col [&>*]:w-full [&>.sr-only]:w-auto @md/field-group:flex-row @md/field-group:items-center @md/field-group:[&>*]:w-auto @md/field-group:[&>[data-slot=field-label]]:flex-auto @md/field-group:has-[>[data-slot=field-content]]:items-start @md/field-group:has-[>[data-slot=field-content]]:[&>[role=checkbox],[role=radio]]:mt-px",
  }
  return cn(base, map[orientation] || map.vertical, options?.className, options?.class)
}

export {
  Field,
  Field as default,
  FieldSet,
  FieldLegend,
  FieldGroup,
  FieldContent,
  FieldLabel,
  FieldTitle,
  FieldDescription,
  FieldSeparator,
  FieldError,
}

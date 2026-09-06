import type { ComputedRef, HTMLAttributes, InjectionKey, Ref } from "vue"
import { cn } from "@/lib/utils"

import Tabs from "./tabs.vue"
import TabsList from "./tabs-list.vue"
import TabsTrigger from "./tabs-trigger.vue"
import TabsContent from "./tabs-content.vue"

export type TabsListVariant = "default" | "line" | "text"

export interface TabsListVariantProps {
  variant?: TabsListVariant | null
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

// tabsListVariants 生成页签列表样式。
export function tabsListVariants(options?: TabsListVariantProps): string {
  const base =
    "group/tabs-list inline-flex w-fit items-center justify-center rounded-lg p-[3px] text-muted-foreground group-data-[orientation=horizontal]/tabs:h-9 group-data-[orientation=vertical]/tabs:h-fit group-data-[orientation=vertical]/tabs:flex-col data-[variant=line]:rounded-none"

  const variantMap: Record<TabsListVariant, string> = {
    default: "bg-muted",
    line: "gap-1 bg-transparent",
    text: "bg-transparent p-0",
  }

  const variant = options?.variant || "default"

  return cn(
    base,
    variantMap[variant] || variantMap.default,
    options?.className,
    options?.class
  )
}

export interface TabsContext {
  currentValue: Ref<string | number>
  setValue: (value: string | number) => void
  orientation: ComputedRef<"horizontal" | "vertical">
}

export const TABS_INJECTION_KEY: InjectionKey<TabsContext> = Symbol("TabsContext")

export {
  Tabs,
  Tabs as default,
  TabsList,
  TabsTrigger,
  TabsContent,
}

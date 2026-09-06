import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import Badge from "./badge.vue"

export type BadgeVariant = "default" | "secondary" | "destructive" | "outline"

export interface BadgeVariantProps {
  variant?: BadgeVariant | null
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

// badgeVariants 生成徽章变体类名映射。
export function badgeVariants(options?: BadgeVariantProps): string {
  const base =
    "inline-flex items-center justify-center rounded-full border px-2 py-0.5 text-xs font-medium w-fit whitespace-nowrap shrink-0 [&>svg]:size-3 gap-1 [&>svg]:pointer-events-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive transition-[color,box-shadow] overflow-hidden"

  const variantMap: Record<BadgeVariant, string> = {
    default:
      "border-transparent bg-primary text-primary-foreground [a&]:hover:bg-primary/90",
    secondary:
      "border-transparent bg-secondary text-secondary-foreground [a&]:hover:bg-secondary/90",
    destructive:
      "border-transparent bg-destructive text-white [a&]:hover:bg-destructive/90 focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40 dark:bg-destructive/60",
    outline:
      "text-foreground [a&]:hover:bg-accent [a&]:hover:text-accent-foreground",
  }

  const variant = options?.variant || "default"

  return cn(
    base,
    variantMap[variant] || variantMap.default,
    options?.className,
    options?.class
  )
}

export { Badge, Badge as default }

<script setup lang="ts">
import type { Component, HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { badgeVariants, type BadgeVariant } from "./badge"

interface Props {
  variant?: BadgeVariant
  as?: string | Component
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  variant: "default",
  as: "span",
})
</script>

<template>
  <component
    :is="as"
    data-slot="badge"
    :class="cn(badgeVariants({ variant }), props.class, props.className)"
  >
    <slot />
  </component>
</template>

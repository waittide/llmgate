<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { tabsListVariants, type TabsListVariant } from "./tabs"

interface Props {
  variant?: TabsListVariant
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  variant: "default",
})
</script>

<template>
  <div
    role="tablist"
    data-slot="tabs-list"
    :data-variant="variant"
    :class="cn(tabsListVariants({ variant }), props.class, props.className)"
  >
    <slot />
  </div>
</template>

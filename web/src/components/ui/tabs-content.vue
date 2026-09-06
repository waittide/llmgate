<script setup lang="ts">
import { computed, inject, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { TABS_INJECTION_KEY } from "./tabs"

interface Props {
  value: string | number
  forceMount?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  forceMount: false,
})

const tabsContext = inject(TABS_INJECTION_KEY, null)

const isActive = computed(() => {
  return tabsContext ? tabsContext.currentValue.value === props.value : false
})
</script>

<template>
  <div
    v-show="forceMount ? true : isActive"
    v-if="forceMount || isActive"
    role="tabpanel"
    data-slot="tabs-content"
    :data-state="isActive ? 'active' : 'inactive'"
    tabindex="0"
    :class="cn('flex-1 outline-none', props.class, props.className)"
  >
    <slot />
  </div>
</template>

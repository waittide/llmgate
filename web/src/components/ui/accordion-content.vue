<script setup lang="ts">
import { computed, inject, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { ACCORDION_ITEM_INJECTION_KEY } from "./accordion"

interface Props {
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = defineProps<Props>()

const item = inject(ACCORDION_ITEM_INJECTION_KEY, null)
const isOpen = computed(() => item?.isOpen.value ?? false)
</script>

<template>
  <div
    data-slot="accordion-content"
    :data-state="isOpen ? 'open' : 'closed'"
    class="overflow-hidden text-sm transition-all duration-200 grid"
    :class="[
      isOpen
        ? 'grid-rows-[1fr] opacity-100'
        : 'grid-rows-[0fr] opacity-0 pointer-events-none',
    ]"
  >
    <div class="overflow-hidden min-h-0">
      <div :class="cn('pt-0 pb-4', props.class, props.className)">
        <slot />
      </div>
    </div>
  </div>
</template>

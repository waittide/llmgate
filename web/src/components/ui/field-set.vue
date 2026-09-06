<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = defineProps<Props>()
</script>

<template>
  <fieldset
    data-slot="field-set"
    :class="
      cn(
        'flex flex-col gap-6 has-[>[data-slot=checkbox-group]]:gap-3 has-[>[data-slot=radio-group]]:gap-3',
        props.class,
        props.className
      )
    "
  >
    <slot />
  </fieldset>
</template>

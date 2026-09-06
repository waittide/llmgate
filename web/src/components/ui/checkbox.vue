<script setup lang="ts">
import { computed, type HTMLAttributes } from "vue"
import { LuCheck } from "vue-icons-plus/lu"
import { cn } from "@/lib/utils"

const modelValue = defineModel<boolean | "indeterminate">()

interface Props {
  checked?: boolean | "indeterminate"
  disabled?: boolean
  required?: boolean
  id?: string
  name?: string
  value?: string
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
})

const emit = defineEmits<{
  "update:checked": [checked: boolean]
  checkedChange: [checked: boolean]
}>()

const isChecked = computed(() => {
  if (modelValue.value !== undefined) {
    return Boolean(modelValue.value)
  }
  return Boolean(props.checked)
})

function toggle() {
  if (props.disabled) return
  const next = !isChecked.value
  modelValue.value = next
  emit("update:checked", next)
  emit("checkedChange", next)
}
</script>

<template>
  <button
    type="button"
    role="checkbox"
    :id="id"
    :name="name"
    :value="value"
    :aria-checked="isChecked ? 'true' : 'false'"
    :aria-required="required ? 'true' : undefined"
    :data-state="isChecked ? 'checked' : 'unchecked'"
    :disabled="disabled"
    data-slot="checkbox"
    :class="
      cn(
        'peer size-4 shrink-0 rounded-[4px] border border-input shadow-xs transition-shadow outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-destructive/20 data-[state=checked]:border-primary data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground dark:bg-input/30 dark:aria-invalid:ring-destructive/40 dark:data-[state=checked]:bg-primary',
        props.class,
        props.className
      )
    "
    @click="toggle"
  >
    <span
      v-if="isChecked"
      data-slot="checkbox-indicator"
      class="grid place-content-center text-current transition-none"
    >
      <LuCheck class="size-3.5" />
    </span>
  </button>
</template>

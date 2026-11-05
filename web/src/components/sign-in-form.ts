/** One field of the sign-in or register card, as the form checks it before sending. */
export interface FieldToCheck {
  id: string;
  /** Already trimmed where spaces do not count (login, username, email); a password is as typed. */
  value: string;
  emptyError: string;
  /** The browser's own reading of the field (`type="email"`), checked once the field is filled. */
  isMalformed?: boolean;
  malformedError?: string;
}

/**
 * Field id to error, in the form's order, so the first key is the field to focus. The form has
 * `novalidate`: these replace the browser's bubbles with Mellow's words (DESIGN.md §5.1).
 */
export function fieldErrorsBeforeSending(fields: readonly FieldToCheck[]): Map<string, string> {
  const errors = new Map<string, string>();
  for (const field of fields) {
    if (field.value === "") {
      errors.set(field.id, field.emptyError);
    } else if (field.isMalformed && field.malformedError) {
      errors.set(field.id, field.malformedError);
    }
  }
  return errors;
}

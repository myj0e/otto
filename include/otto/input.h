#ifndef OTTO_INPUT_H
#define OTTO_INPUT_H

int otto_input_is_interactive(void);
int otto_input_read_line(
    const char *prompt,
    const char *default_value,
    char **result
);
int otto_input_read_secret(const char *prompt, char **result);
int otto_input_confirm(const char *prompt, int default_yes);

#endif

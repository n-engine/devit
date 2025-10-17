#ifndef GOOD_EXAMPLE_H
#define GOOD_EXAMPLE_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_NAME_LENGTH 50
#define BUFFER_SIZE 100

/**
 * Safely reads user input and processes it
 * @param buffer Destination buffer for input
 * @param size Maximum size of buffer
 * @return 0 on success, -1 on error
 */
int read_user_input(char *buffer, size_t size) {
    printf("Enter your name: ");
    
    if (fgets(buffer, size, stdin) == NULL) {
        return -1;
    }
    
    // Remove newline if present
    size_t len = strlen(buffer);
    if (len > 0 && buffer[len - 1] == '\n') {
        buffer[len - 1] = '\0';
    }
    
    return 0;
}

/**
 * Processes the user input safely
 * @param input The input string to process
 * @return 0 on success, -1 on error
 */
int process_input(const char *input) {
    if (input == NULL) {
        return -1;
    }
    
    size_t input_len = strlen(input);
    if (input_len > MAX_NAME_LENGTH) {
        fprintf(stderr, "Input too long (max %d characters)\n", 
                MAX_NAME_LENGTH);
        return -1;
    }
    
    char *processed = malloc(input_len + 10);
    if (processed == NULL) {
        fprintf(stderr, "Memory allocation failed\n");
        return -1;
    }
    
    snprintf(processed, input_len + 10, "Hello, %s!", input);
    printf("%s\n", processed);
    
    free(processed);
    processed = NULL;
    
    return 0;
}

int main(void) {
    char buffer[BUFFER_SIZE];
    
    if (read_user_input(buffer, sizeof(buffer)) != 0) {
        fprintf(stderr, "Failed to read input\n");
        return 1;
    }
    
    if (process_input(buffer) != 0) {
        fprintf(stderr, "Failed to process input\n");
        return 1;
    }
    
    return 0;
}

#endif // GOOD_EXAMPLE_H
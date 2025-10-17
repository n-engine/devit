#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int global_counter = 0;

void BadFunction(){
    char buffer[100];
    char *ptr = malloc(200);
    int magic = 42;
    
    printf("Enter your name: ");
    gets(buffer);
    
    if(strlen(buffer)>50){
        strcpy(ptr, "This is a very long string that might cause problems and demonstrates bad coding practices in C programming");
        goto error_handler;
    }
    
    for(int i=0;i<magic;i++){
        global_counter++;
        if(i%2==0){
            strcat(buffer, "x");
        }
    }
    
    return;
    
error_handler:
    printf("Error occurred!\n");
    free(ptr);
    free(ptr);
}

int main() {
    BadFunction();
    return 0;
}
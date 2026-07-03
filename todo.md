1. will be using git cli for git operations.
2. .env needs to have:
  local git directory from the root do not include ~ example just .local/state/tracker, 
  and the git origin command.
3. init operation will be done if the repo does not contain a .git and if the .git corresponds to a different name than the github repo
 (dont know if this can be done)
6. Report
  1. a service that autoruns once a day and uses the data from the day before to generate report.
  2. the generated report can then be displayed somewhere.
  3. can also be done manually if want to see report before end of day.
7. installation script:
  1. clone and install the software.
  2. run setup keyboard.sh and run usermod -aG input $USER  
  3. once keyboard identified add to env and ask user to input other necessary env data.
  4. cargo build and add binary to global path.
  5. create a systemctl service that start daemon everytime the computer boots.
  6. run tracker init to initialize the github repo (this runs only once)
  7. check if git cli is configured and user.name and user.email exists else ask the user to rerun after setting up git.
 
8. hourly csv  

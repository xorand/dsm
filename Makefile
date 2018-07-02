OBJ=dsm

all:
	cargo build --release
	mkdir -p run
	service supervisor stop
	while pgrep -u root $(OBJ); do sleep 1; done
	cp target/release/$(OBJ) run/
	strip run/$(OBJ)
	service supervisor start

config:
	cp $(OBJ).ini run/

OBJ=dsm

all:
	cargo build --release
	mkdir -p run
	service supervisor stop
	sleep 1
	cp target/release/$(OBJ) run/
	strip run/$(OBJ)
	service supervisor start

config:
	cp $(OBJ).ini run/

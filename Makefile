all: gpio.json spi.json uart.json timer.json

appendix.pdf: esp8266-technical_reference_en.pdf
	qpdf esp8266-technical_reference_en.pdf --pages . 113-116 -- appendix.pdf

tabula.jar:
	wget https://github.com/tabulapdf/tabula-java/releases/download/v1.0.3/tabula-1.0.3-jar-with-dependencies.jar -O tabula.jar

gpio.json: appendix.pdf tabula.jar
	java -jar tabula.jar -p 1 -l -f JSON appendix.pdf -o gpio.json

spi.json: appendix.pdf tabula.jar
	java -jar tabula.jar -p 2 -l -f JSON appendix.pdf -o spi.json

uart.json: appendix.pdf tabula.jar
	java -jar tabula.jar -p 3 -l -f JSON appendix.pdf -o uart.json

timer.json: appendix.pdf tabula.jar
	java -jar tabula.jar -p 4 -l -f JSON appendix.pdf -o timer.json
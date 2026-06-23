import yaml, os, sys
d = sys.argv[1]
descr = yaml.safe_load(open(os.path.join(d, "descriptor.yaml")))
for f in descr["compile"]["verilogSourceFiles"]:
    print(os.path.join(d, f))

FROM public.ecr.aws/lambda/python:3.14

WORKDIR ${LAMBDA_TASK_ROOT}

COPY pyproject.toml ./
RUN python -c "import pathlib, subprocess, sys, tomllib; deps = tomllib.loads(pathlib.Path('pyproject.toml').read_text())['project']['dependencies']; subprocess.check_call([sys.executable, '-m', 'pip', 'install', '--no-cache-dir', *deps])"

COPY graph.py main.py tools.py ./

CMD ["main.lambda_handler"]
